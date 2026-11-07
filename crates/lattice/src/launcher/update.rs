//! Finding and installing a newer Lattice. Releases live on GitHub, each with the bare
//! program for every system and its checksum. Only what's needed to find and fetch them is
//! sent: the request says it's Lattice, and nothing about who's asking.

use crossbeam_channel::{Receiver, unbounded};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const LATEST: &str = "https://api.github.com/repos/DefNotIntoWaifus/lattice/releases/latest";
const MAX_PROGRAM: u64 = 512 * 1024 * 1024;

/// Which build of the program this is, as release files are named.
pub const fn target() -> &'static str {
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "linux-x86_64"
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        "windows-x86_64"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "macos-arm64"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "macos-x86_64"
    } else {
        "unknown"
    }
}

fn asset_name(version: &str) -> String {
    let ext = if cfg!(windows) { ".exe" } else { "" };
    format!("lattice-{version}-{}{ext}", target())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    /// What it's called, which says the Minecraft version it's for.
    pub title: String,
    pub page: String,
    program_url: String,
    checksum_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Update {
    /// Not looked yet, or there's nothing newer.
    None,
    Available(Release),
    Downloading(Release),
    /// In place; takes effect on the next start.
    Installed(String),
    Failed(String),
}

pub struct Updater {
    pub state: Update,
    rx: Option<Receiver<Update>>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    #[serde(default)]
    name: Option<String>,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<Asset>,
}

/// `0.10.2` as numbers, so that it counts as newer than `0.9`.
fn parts(version: &str) -> Vec<u32> {
    version.trim_start_matches('v').split('.').map(|p| p.split(|c: char| !c.is_ascii_digit()).next().and_then(|n| n.parse().ok()).unwrap_or(0)).collect()
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    let (a, b) = (parts(candidate), parts(current));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    false
}

fn read_release(body: &str, current: &str) -> Option<Release> {
    let r: GithubRelease = serde_json::from_str(body).ok()?;
    if r.draft || r.prerelease {
        return None;
    }
    let version = r.tag_name.trim_start_matches('v').to_string();
    if !is_newer(&version, current) {
        return None;
    }
    let wanted = asset_name(&version);
    let checksum = format!("{wanted}.sha256");
    let find = |name: &str| r.assets.iter().find(|a| a.name == name).map(|a| a.browser_download_url.clone());
    Some(Release { title: r.name.unwrap_or_else(|| format!("Lattice {version}")), version, page: r.html_url, program_url: find(&wanted)?, checksum_url: find(&checksum)? })
}

fn get(url: &str) -> Result<ureq::http::Response<ureq::Body>, String> {
    ureq::get(url).header("User-Agent", concat!("lattice/", env!("CARGO_PKG_VERSION"))).header("Accept", "application/vnd.github+json").call().map_err(|e| e.to_string())
}

fn check_at(url: &str, current: &str) -> Option<Release> {
    let body = get(url).ok()?.into_body().read_to_string().ok()?;
    read_release(&body, current)
}

/// Whether a file's what its checksum line says it is. Lines read `<hex>  <name>`.
fn checksum_matches(bytes: &[u8], line: &str) -> bool {
    let wanted = line.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
    let got: String = Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect();
    wanted.len() == 64 && wanted == got
}

/// Replaces the program at `program`. The new one goes in beside it and takes its name in a
/// single step, so a failure at any point leaves the old one working.
fn replace_program(program: &Path, bytes: &[u8]) -> Result<(), String> {
    let new = program.with_extension("update");
    std::fs::write(&new, bytes).map_err(|e| format!("Couldn't write the update: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o755)).map_err(|e| format!("Couldn't make the update runnable: {e}"))?;
    }
    // Windows won't overwrite a running program, but it will let it be moved aside.
    #[cfg(windows)]
    {
        let old = program.with_extension("old");
        let _ = std::fs::remove_file(&old);
        std::fs::rename(program, &old).map_err(|e| format!("Couldn't move the old program aside: {e}"))?;
    }
    std::fs::rename(&new, program).map_err(|e| format!("Couldn't put the update in place: {e}"))
}

fn download(release: &Release, program: &PathBuf) -> Update {
    let line = match get(&release.checksum_url).and_then(|r| r.into_body().read_to_string().map_err(|e| e.to_string())) {
        Ok(line) => line,
        Err(e) => return Update::Failed(format!("Couldn't get the update's checksum: {e}")),
    };
    let bytes = match get(&release.program_url).and_then(|r| r.into_body().with_config().limit(MAX_PROGRAM).read_to_vec().map_err(|e| e.to_string())) {
        Ok(bytes) => bytes,
        Err(e) => return Update::Failed(format!("Couldn't download the update: {e}")),
    };
    if !checksum_matches(&bytes, &line) {
        return Update::Failed("The download didn't match its checksum, so it wasn't installed.".to_string());
    }
    match replace_program(program, &bytes) {
        Ok(()) => Update::Installed(release.version.clone()),
        Err(e) => Update::Failed(e),
    }
}

impl Updater {
    pub fn new() -> Updater {
        Updater { state: Update::None, rx: None }
    }

    /// Looks for a newer version in the background. A failure to look is no one's problem:
    /// offline, or a repository that isn't public, simply means nothing's offered.
    pub fn check(&mut self) {
        let (tx, rx) = unbounded();
        self.rx = Some(rx);
        let url = std::env::var("LATTICE_UPDATE_URL").unwrap_or_else(|_| LATEST.to_string());
        let spawned = std::thread::Builder::new().name("update-check".into()).spawn(move || {
            if let Some(release) = check_at(&url, env!("CARGO_PKG_VERSION")) {
                let _ = tx.send(Update::Available(release));
            }
        });
        if let Err(e) = spawned {
            tracing::warn!("couldn't look for updates: {e}");
        }
    }

    pub fn install(&mut self) {
        let Update::Available(release) = self.state.clone() else { return };
        let program = match std::env::current_exe() {
            Ok(p) => p,
            Err(e) => {
                self.state = Update::Failed(format!("Couldn't find this program on disk: {e}"));
                return;
            }
        };
        self.state = Update::Downloading(release.clone());
        let (tx, rx) = unbounded();
        self.rx = Some(rx);
        let spawned = std::thread::Builder::new().name("update-install".into()).spawn(move || {
            let _ = tx.send(download(&release, &program));
        });
        if let Err(e) = spawned {
            self.state = Update::Failed(e.to_string());
        }
    }

    /// Takes in what the background work's found. True if anything changed.
    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.rx else { return false };
        match rx.try_recv() {
            Ok(state) => {
                self.state = state;
                true
            }
            Err(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[test]
    fn newer_is_by_number_not_by_text() {
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("v0.2.1", "0.2.0"));
        assert!(is_newer("1.0", "0.9.9"));
        assert!(!is_newer("0.2.0", "0.2.0"));
        assert!(!is_newer("0.2", "0.2.0"));
        assert!(!is_newer("0.1.9", "0.2.0"));
    }

    fn release_json(tag: &str, draft: bool, prerelease: bool) -> String {
        let name = asset_name(tag.trim_start_matches('v'));
        format!(r#"{{"tag_name":"{tag}","name":"Lattice {tag} for Minecraft 26.4","html_url":"https://example.com/r","draft":{draft},"prerelease":{prerelease},"assets":[{{"name":"{name}","browser_download_url":"https://example.com/p"}},{{"name":"{name}.sha256","browser_download_url":"https://example.com/c"}}]}}"#)
    }

    #[test]
    fn only_finished_newer_releases_with_this_systems_program_count() {
        let r = read_release(&release_json("v9.1.0", false, false), "0.2.0").unwrap();
        assert_eq!(r.version, "9.1.0");
        assert!(r.title.contains("26.4"));
        assert!(read_release(&release_json("v0.2.0", false, false), "0.2.0").is_none());
        assert!(read_release(&release_json("v9.1.0", true, false), "0.2.0").is_none());
        assert!(read_release(&release_json("v9.1.0", false, true), "0.2.0").is_none());
        let other = r#"{"tag_name":"v9.1.0","html_url":"x","assets":[{"name":"lattice-9.1.0-plan9","browser_download_url":"y"}]}"#;
        assert!(read_release(other, "0.2.0").is_none());
    }

    #[test]
    fn a_bad_checksum_is_refused() {
        // The SHA-256 of "abc", from the published test vectors.
        let good = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  file";
        assert!(checksum_matches(b"abc", good));
        assert!(!checksum_matches(b"abd", good));
        assert!(!checksum_matches(b"abc", "not a checksum"));
        assert!(!checksum_matches(b"abc", ""));
    }

    /// Answers each request in `files` once, by the path it asks for.
    fn serve(files: Vec<(&'static str, Vec<u8>)>) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming().take(files.len() * 2) {
                let Ok(mut stream) = stream else { continue };
                let mut request = [0u8; 2048];
                let n = stream.read(&mut request).unwrap_or(0);
                let text = String::from_utf8_lossy(&request[..n]);
                let path = text.split_whitespace().nth(1).unwrap_or("/").to_string();
                let body = files.iter().find(|(p, _)| *p == path).map(|(_, b)| b.clone()).unwrap_or_default();
                let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
            }
        });
        address
    }

    fn temp_program(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lattice-update-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let program = dir.join("lattice");
        std::fs::write(&program, b"old program").unwrap();
        program
    }

    #[test]
    fn an_update_downloads_checks_and_takes_the_programs_place() {
        let new = b"new program".to_vec();
        let sum: String = Sha256::digest(&new).iter().map(|b| format!("{b:02x}")).collect();
        let base = serve(vec![("/program", new.clone()), ("/sum", format!("{sum}  lattice").into_bytes())]);
        let release = Release { version: "9.0.0".into(), title: "t".into(), page: "p".into(), program_url: format!("{base}/program"), checksum_url: format!("{base}/sum") };
        let program = temp_program("good");
        assert_eq!(download(&release, &program), Update::Installed("9.0.0".into()));
        assert_eq!(std::fs::read(&program).unwrap(), new);
        std::fs::remove_dir_all(program.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_corrupt_download_leaves_the_old_program_alone() {
        let wrong = "0".repeat(64);
        let base = serve(vec![("/program", b"tampered".to_vec()), ("/sum", format!("{wrong}  lattice").into_bytes())]);
        let release = Release { version: "9.0.0".into(), title: "t".into(), page: "p".into(), program_url: format!("{base}/program"), checksum_url: format!("{base}/sum") };
        let program = temp_program("bad");
        assert!(matches!(download(&release, &program), Update::Failed(_)));
        assert_eq!(std::fs::read(&program).unwrap(), b"old program");
        std::fs::remove_dir_all(program.parent().unwrap()).unwrap();
    }
}
