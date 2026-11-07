//! What it takes to run a world on this computer: Mojang's server program and the Java it needs,
//! both downloaded from Mojang and checked against the sums Mojang publishes.

use crate::{AssetError, Progress, agent, get_json, sha1_file};
use serde::Deserialize;
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

const JAVA_LIST: &str = "https://launchermeta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json";

#[derive(Deserialize)]
struct Version {
    downloads: Downloads,
    #[serde(rename = "javaVersion")]
    java: JavaVersion,
}

#[derive(Deserialize)]
struct Downloads {
    server: crate::Download,
}

#[derive(Deserialize)]
struct JavaVersion {
    component: String,
}

#[derive(Deserialize)]
struct RuntimeEntry {
    manifest: Pointer,
}

#[derive(Deserialize)]
struct Pointer {
    sha1: String,
    url: String,
}

#[derive(Deserialize)]
struct RuntimeManifest {
    files: HashMap<String, RuntimeFile>,
}

#[derive(Deserialize)]
struct RuntimeFile {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    executable: bool,
    downloads: Option<RuntimeDownloads>,
    target: Option<String>,
}

#[derive(Deserialize)]
struct RuntimeDownloads {
    raw: crate::Download,
}

/// What Mojang calls this computer in its list of runtimes.
fn platform() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "linux",
        ("linux", "x86") => "linux-i386",
        ("macos", "x86_64") => "mac-os",
        ("macos", "aarch64") => "mac-os-arm64",
        ("windows", "x86_64") => "windows-x64",
        ("windows", "x86") => "windows-x86",
        ("windows", "aarch64") => "windows-arm64",
        _ => return None,
    })
}

pub struct ServerFiles {
    pub jar: PathBuf,
    pub java: PathBuf,
}

fn io_err(path: &Path) -> impl FnOnce(std::io::Error) -> AssetError {
    let path = path.to_path_buf();
    move |source| AssetError::Io { path, source }
}

fn download_to(url: &str, sha1: &str, size: u64, path: &Path, mut progress: impl FnMut(u64)) -> Result<(), AssetError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(io_err(dir))?;
    }
    let part = path.with_extension("part");
    let mut last = None;
    for _ in 0..3 {
        let result = (|| -> Result<(), AssetError> {
            let mut resp = agent().get(url).call().map_err(|e| AssetError::Download { url: url.to_string(), reason: e.to_string() })?;
            let mut reader = resp.body_mut().with_config().limit(size + 1024).reader();
            let mut out = std::fs::File::create(&part).map_err(io_err(&part))?;
            let mut hash = Sha1::new();
            let mut buf = vec![0u8; 1 << 16];
            loop {
                let n = std::io::Read::read(&mut reader, &mut buf).map_err(|e| AssetError::Download { url: url.to_string(), reason: e.to_string() })?;
                if n == 0 {
                    break;
                }
                hash.update(&buf[..n]);
                std::io::Write::write_all(&mut out, &buf[..n]).map_err(io_err(&part))?;
                progress(n as u64);
            }
            let got: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
            if got != sha1 {
                return Err(AssetError::Checksum { url: url.to_string() });
            }
            std::fs::rename(&part, path).map_err(io_err(path))
        })();
        match result {
            Ok(()) => return Ok(()),
            Err(e) => last = Some(e),
        }
    }
    let _ = std::fs::remove_file(&part);
    Err(last.unwrap_or(AssetError::Archive("the download never started".into())))
}

/// Downloads whatever's missing. `progress` is told which step it's on and how far along.
pub fn ensure_server(cache: &Path, progress: &(dyn Fn(&str, Progress) + Sync)) -> Result<ServerFiles, AssetError> {
    let manifest: crate::Manifest = get_json(crate::MANIFEST_URL)?;
    let entry = manifest.versions.iter().find(|v| v.id == crate::GAME_VERSION).ok_or_else(|| AssetError::UnknownVersion(crate::GAME_VERSION.into()))?;
    let version: Version = get_json(&entry.url)?;

    // The server program.
    let jar = cache.join("versions").join(crate::GAME_VERSION).join("server.jar");
    let d = &version.downloads.server;
    if sha1_file(&jar).as_deref() != Some(d.sha1.as_str()) {
        let total = d.size;
        let mut done = 0;
        download_to(&d.url, &d.sha1, d.size, &jar, |n| {
            done += n;
            progress("Downloading the server program", Progress { done, total: Some(total) });
        })?;
    }

    // The Java it runs on.
    let platform = platform().ok_or_else(|| AssetError::Archive(format!("Mojang doesn't publish Java for {} {}", std::env::consts::OS, std::env::consts::ARCH)))?;
    let list: HashMap<String, HashMap<String, Vec<RuntimeEntry>>> = get_json(JAVA_LIST)?;
    let runtime = list
        .get(platform)
        .and_then(|p| p.get(&version.java.component))
        .and_then(|v| v.first())
        .ok_or_else(|| AssetError::Archive(format!("Mojang lists no {} for {platform}", version.java.component)))?;
    let dir = cache.join("java").join(&version.java.component);
    let java = dir.join(if cfg!(windows) { "bin/javaw.exe" } else { "bin/java" });
    let java = match std::env::consts::OS {
        // Packaged differently there.
        "macos" => dir.join("jre.bundle/Contents/Home/bin/java"),
        _ => java,
    };
    let done_marker = dir.join(".complete");
    if std::fs::read_to_string(&done_marker).ok().as_deref() != Some(runtime.manifest.sha1.as_str()) || !java.exists() {
        let files: RuntimeManifest = get_json(&runtime.manifest.url)?;
        install_runtime(&dir, &files, progress)?;
        std::fs::write(&done_marker, &runtime.manifest.sha1).map_err(io_err(&done_marker))?;
    }
    Ok(ServerFiles { jar, java })
}

fn install_runtime(dir: &Path, manifest: &RuntimeManifest, progress: &(dyn Fn(&str, Progress) + Sync)) -> Result<(), AssetError> {
    std::fs::create_dir_all(dir).map_err(io_err(dir))?;
    let total: u64 = manifest.files.values().filter_map(|f| f.downloads.as_ref()).map(|d| d.raw.size).sum();
    for (name, f) in &manifest.files {
        if f.kind == "directory" {
            let p = dir.join(name);
            std::fs::create_dir_all(&p).map_err(io_err(&p))?;
        }
    }
    let queue: Mutex<Vec<(&String, &RuntimeFile)>> = Mutex::new(manifest.files.iter().filter(|(_, f)| f.kind == "file").collect());
    let done = AtomicU64::new(0);
    let failure: Mutex<Option<AssetError>> = Mutex::new(None);
    std::thread::scope(|scope| {
        for _ in 0..6 {
            scope.spawn(|| {
                loop {
                    let next = queue.lock().unwrap_or_else(|e| e.into_inner()).pop();
                    let Some((name, f)) = next else { return };
                    let Some(d) = &f.downloads else { continue };
                    let path = dir.join(name);
                    let result = if sha1_file(&path).as_deref() == Some(d.raw.sha1.as_str()) {
                        done.fetch_add(d.raw.size, Ordering::Relaxed);
                        Ok(())
                    } else {
                        download_to(&d.raw.url, &d.raw.sha1, d.raw.size, &path, |n| {
                            let so_far = done.fetch_add(n, Ordering::Relaxed) + n;
                            progress("Downloading Java", Progress { done: so_far, total: Some(total) });
                        })
                    };
                    let result = result.and_then(|()| mark_executable(&path, f.executable));
                    if let Err(e) = result {
                        *failure.lock().unwrap_or_else(|e| e.into_inner()) = Some(e);
                        return;
                    }
                }
            });
        }
    });
    if let Some(e) = failure.into_inner().unwrap_or_else(|e| e.into_inner()) {
        return Err(e);
    }
    for (name, f) in &manifest.files {
        if let (true, Some(target)) = (f.kind == "link", &f.target) {
            make_link(dir, name, target)?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn mark_executable(path: &Path, executable: bool) -> Result<(), AssetError> {
    use std::os::unix::fs::PermissionsExt;
    if executable {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).map_err(io_err(path))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn mark_executable(_path: &Path, _executable: bool) -> Result<(), AssetError> {
    Ok(())
}

#[cfg(unix)]
fn make_link(dir: &Path, name: &str, target: &str) -> Result<(), AssetError> {
    let path = dir.join(name);
    // Links only ever point within the runtime; one that doesn't isn't followed.
    let parent = path.parent().unwrap_or(dir);
    let resolved = parent.join(target);
    let mut depth = 0i32;
    for part in Path::new(name).parent().into_iter().flat_map(|p| p.components()) {
        let _ = part;
        depth += 1;
    }
    for part in Path::new(target).components() {
        match part {
            std::path::Component::ParentDir => depth -= 1,
            std::path::Component::Normal(_) => depth += 1,
            _ => depth = -1,
        }
        if depth < 0 {
            return Err(AssetError::Archive(format!("a link in the Java runtime points outside it: {name} to {}", resolved.display())));
        }
    }
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).map_err(io_err(p))?;
    }
    let _ = std::fs::remove_file(&path);
    std::os::unix::fs::symlink(target, &path).map_err(io_err(&path))
}

#[cfg(not(unix))]
fn make_link(_dir: &Path, _name: &str, _target: &str) -> Result<(), AssetError> {
    Ok(())
}
