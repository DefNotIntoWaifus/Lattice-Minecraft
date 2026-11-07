//! Game resources (block models, textures, translations).
//!
//! Lattice ships none of Mojang's art. On first use it downloads the official
//! `client.jar` for the supported version from Mojang's CDN, checks the SHA-1
//! listed in Mojang's version manifest, and caches it. Only files under
//! `assets/` are ever read from it; no code is loaded.

pub mod classvm;
pub mod geometry;
pub mod runtime;

use serde::Deserialize;
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const GAME_VERSION: &str = "26.3";
pub(crate) const MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

#[derive(Debug, thiserror::Error)]
pub enum AssetError {
    #[error("couldn't download {url}: {reason}")]
    Download { url: String, reason: String },
    #[error("the download from {url} was corrupted (checksum mismatch)")]
    Checksum { url: String },
    #[error("Mojang's version list doesn't contain {0}")]
    UnknownVersion(String),
    #[error("couldn't write to the cache folder {path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("the cached game files are unreadable: {0}")]
    Archive(String),
}

#[derive(Debug, Clone, Copy)]
pub struct Progress {
    pub done: u64,
    pub total: Option<u64>,
}

#[derive(Deserialize)]
pub(crate) struct Manifest {
    pub(crate) versions: Vec<ManifestVersion>,
}

#[derive(Deserialize)]
pub(crate) struct ManifestVersion {
    pub(crate) id: String,
    pub(crate) url: String,
}

#[derive(Deserialize)]
struct VersionJson {
    downloads: Downloads,
}

#[derive(Deserialize)]
struct Downloads {
    client: Download,
}

#[derive(Deserialize)]
pub(crate) struct Download {
    pub(crate) sha1: String,
    pub(crate) size: u64,
    pub(crate) url: String,
}

/// One client for everything, so downloads share connections rather than opening a new
/// one for each small file.
pub(crate) fn agent() -> &'static ureq::Agent {
    static AGENT: std::sync::OnceLock<ureq::Agent> = std::sync::OnceLock::new();
    AGENT.get_or_init(ureq::Agent::new_with_defaults)
}

fn get(url: &str) -> Result<ureq::http::Response<ureq::Body>, AssetError> {
    agent()
        .get(url)
        .header("User-Agent", concat!("lattice/", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(|e| AssetError::Download { url: url.to_string(), reason: e.to_string() })
}

pub(crate) fn get_json<T: serde::de::DeserializeOwned>(url: &str) -> Result<T, AssetError> {
    get(url)?
        .body_mut()
        .with_config()
        .limit(16 * 1024 * 1024)
        .read_json()
        .map_err(|e| AssetError::Download { url: url.to_string(), reason: e.to_string() })
}

pub(crate) fn sha1_file(path: &Path) -> Option<String> {
    let mut f = File::open(path).ok()?;
    let mut h = Sha1::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Some(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Path of the verified client jar, downloading it if needed.
pub fn ensure_client_jar(cache: &Path, progress: &dyn Fn(Progress)) -> Result<PathBuf, AssetError> {
    let dir = cache.join("versions").join(GAME_VERSION);
    let jar = dir.join("client.jar");
    let sha_file = dir.join("client.jar.sha1");
    let io = |path: &Path| {
        let path = path.to_path_buf();
        move |source| AssetError::Io { path, source }
    };
    if let (Ok(expected), true) = (std::fs::read_to_string(&sha_file), jar.exists()) {
        // Trust the cache only if the stored checksum still matches the file.
        if sha1_file(&jar).as_deref() == Some(expected.trim()) {
            return Ok(jar);
        }
        tracing::warn!("cached client.jar failed its checksum; downloading again");
    }
    std::fs::create_dir_all(&dir).map_err(io(&dir))?;

    let manifest: Manifest = get_json(MANIFEST_URL)?;
    let entry = manifest
        .versions
        .iter()
        .find(|v| v.id == GAME_VERSION)
        .ok_or_else(|| AssetError::UnknownVersion(GAME_VERSION.into()))?;
    let version: VersionJson = get_json(&entry.url)?;
    let d = version.downloads.client;

    let mut last_err = None;
    for attempt in 0..3 {
        if attempt > 0 {
            tracing::warn!("retrying client.jar download ({attempt})");
        }
        let part = dir.join("client.jar.part");
        let result = (|| -> Result<(), AssetError> {
            let mut resp = get(&d.url)?;
            let total = resp.body().content_length().or(Some(d.size));
            let mut reader = resp.body_mut().with_config().limit(d.size + 1024).reader();
            let mut out = File::create(&part).map_err(io(&part))?;
            let mut h = Sha1::new();
            let mut buf = vec![0u8; 1 << 16];
            let mut done = 0u64;
            loop {
                let n = reader
                    .read(&mut buf)
                    .map_err(|e| AssetError::Download { url: d.url.clone(), reason: e.to_string() })?;
                if n == 0 {
                    break;
                }
                h.update(&buf[..n]);
                out.write_all(&buf[..n]).map_err(io(&part))?;
                done += n as u64;
                progress(Progress { done, total });
            }
            out.sync_all().map_err(io(&part))?;
            let got: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
            if got != d.sha1 {
                return Err(AssetError::Checksum { url: d.url.clone() });
            }
            std::fs::rename(&part, &jar).map_err(io(&jar))?;
            std::fs::write(&sha_file, &d.sha1).map_err(io(&sha_file))?;
            Ok(())
        })();
        match result {
            Ok(()) => return Ok(jar),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap())
}

/// Read access to resource files, with optional resource packs layered on top.
pub struct Resources {
    jar: Mutex<zip::ZipArchive<File>>,
    /// Resource packs, lowest first; a higher one's file wins.
    packs: Vec<Pack>,
    names: Vec<String>,
}

/// Biggest single file taken from a pack, so a crafted zip can't exhaust memory.
const MAX_PACK_FILE: u64 = 64 * 1024 * 1024;

enum Pack {
    Zip(Mutex<zip::ZipArchive<File>>),
    Folder(PathBuf),
}

impl Pack {
    fn open(path: &Path) -> Result<Pack, AssetError> {
        if path.is_dir() {
            return Ok(Pack::Folder(path.to_path_buf()));
        }
        let file = File::open(path).map_err(|e| AssetError::Io { path: path.to_path_buf(), source: e })?;
        let archive = zip::ZipArchive::new(file).map_err(|e| AssetError::Archive(e.to_string()))?;
        Ok(Pack::Zip(Mutex::new(archive)))
    }

    fn read(&self, full: &str) -> Option<Vec<u8>> {
        match self {
            Pack::Zip(zip) => {
                let mut zip = zip.lock().unwrap();
                let f = zip.by_name(full).ok()?;
                if f.size() > MAX_PACK_FILE {
                    return None;
                }
                let mut buf = Vec::new();
                f.take(MAX_PACK_FILE).read_to_end(&mut buf).ok()?;
                Some(buf)
            }
            // Names come from this program, never from the pack, so they stay inside it.
            Pack::Folder(dir) => {
                let path = dir.join(full);
                if std::fs::metadata(&path).ok()?.len() > MAX_PACK_FILE {
                    return None;
                }
                std::fs::read(path).ok()
            }
        }
    }

    fn names(&self) -> Vec<String> {
        match self {
            Pack::Zip(zip) => zip.lock().unwrap().file_names().filter_map(Result::ok).filter(|n| n.starts_with("assets/")).map(|n| n.into_owned()).collect(),
            Pack::Folder(dir) => {
                let mut out = Vec::new();
                let mut stack = vec![dir.join("assets")];
                while let Some(d) = stack.pop() {
                    let Ok(entries) = std::fs::read_dir(&d) else { continue };
                    for e in entries.flatten() {
                        let p = e.path();
                        if p.is_dir() {
                            stack.push(p);
                        } else if let Ok(rel) = p.strip_prefix(dir) {
                            out.push(rel.to_string_lossy().replace('\\', "/"));
                        }
                    }
                }
                out
            }
        }
    }
}

impl Resources {
    pub fn open(jar: &Path) -> Result<Self, AssetError> {
        let file = File::open(jar).map_err(|e| AssetError::Io { path: jar.to_path_buf(), source: e })?;
        let archive = zip::ZipArchive::new(file).map_err(|e| AssetError::Archive(e.to_string()))?;
        let names = archive
            .file_names()
            .filter_map(Result::ok)
            .filter(|n| n.starts_with("assets/"))
            .map(|n| n.into_owned())
            .collect();
        Ok(Resources { jar: Mutex::new(archive), packs: Vec::new(), names })
    }

    /// The game's files with resource packs over them, lowest first. A pack that
    /// can't be read is skipped and reported, and the rest still apply.
    pub fn open_with_packs(jar: &Path, packs: &[PathBuf]) -> Result<Self, AssetError> {
        let mut res = Self::open(jar)?;
        for path in packs {
            match Pack::open(path) {
                Ok(pack) => {
                    res.names.extend(pack.names());
                    res.packs.push(pack);
                }
                Err(e) => tracing::warn!("skipping resource pack {}: {e}", path.display()),
            }
        }
        res.names.sort();
        res.names.dedup();
        Ok(res)
    }

    /// Reads `assets/minecraft/<path>`, from the highest pack that has it.
    pub fn read(&self, path: &str) -> Option<Vec<u8>> {
        let full = format!("assets/minecraft/{path}");
        if let Some(buf) = self.packs.iter().rev().find_map(|p| p.read(&full)) {
            return Some(buf);
        }
        self.read_jar(&full)
    }

    fn read_jar(&self, full: &str) -> Option<Vec<u8>> {
        let mut jar = self.jar.lock().unwrap();
        let mut f = jar.by_name(full).ok()?;
        let mut buf = Vec::with_capacity(f.size() as usize);
        f.read_to_end(&mut buf).ok()?;
        Some(buf)
    }

    pub fn json(&self, path: &str) -> Option<serde_json::Value> {
        serde_json::from_slice(&self.read(path)?).ok()
    }

    pub fn image(&self, path: &str) -> Option<image::RgbaImage> {
        let full = format!("assets/minecraft/{path}");
        let decode = |bytes: Vec<u8>| image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).ok().map(|i| i.to_rgba8());
        // A broken picture in a pack falls through to the one below it.
        self.packs.iter().rev().find_map(|p| p.read(&full).and_then(decode)).or_else(|| self.read_jar(&full).and_then(decode))
    }

    /// Paths (relative to `assets/minecraft/`) under a directory with an extension.
    pub fn list(&self, dir: &str, ext: &str) -> Vec<String> {
        let prefix = format!("assets/minecraft/{dir}");
        self.names
            .iter()
            .filter(|n| n.starts_with(&prefix) && n.ends_with(ext))
            .map(|n| n["assets/minecraft/".len()..].to_string())
            .collect()
    }

    /// The game's English translations (`lang/en_us.json`).
    pub fn translations(&self, locale: &str) -> HashMap<String, String> {
        self.json(&format!("lang/{locale}.json"))
            .or_else(|| self.json("lang/en_us.json"))
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default()
    }
}

/// Sounds and other files that aren't in the client jar: fetched from Mojang by content hash the
/// first time they're wanted, checked against their hash, and kept in the cache.
pub struct AssetStore {
    objects: HashMap<String, (String, u64)>,
    dir: PathBuf,
}

#[derive(Deserialize)]
struct AssetIndexRef {
    id: String,
    url: String,
    sha1: String,
}

#[derive(Deserialize)]
struct VersionAssets {
    #[serde(rename = "assetIndex")]
    asset_index: AssetIndexRef,
}

#[derive(Deserialize)]
struct AssetObject {
    hash: String,
    size: u64,
}

#[derive(Deserialize)]
struct AssetIndexFile {
    objects: HashMap<String, AssetObject>,
}

const OBJECTS_URL: &str = "https://resources.download.minecraft.net";

impl AssetStore {
    /// Opens the list of files for this version, downloading the list if it isn't cached.
    pub fn open(cache: &Path) -> Result<AssetStore, AssetError> {
        let dir = cache.join("assets");
        let index_path = dir.join("indexes").join(format!("{GAME_VERSION}.json"));
        let io = |path: &Path| {
            let path = path.to_path_buf();
            move |source| AssetError::Io { path, source }
        };
        let bytes = match std::fs::read(&index_path) {
            Ok(b) => b,
            Err(_) => {
                let manifest: Manifest = get_json(MANIFEST_URL)?;
                let entry = manifest.versions.iter().find(|v| v.id == GAME_VERSION).ok_or_else(|| AssetError::UnknownVersion(GAME_VERSION.into()))?;
                let version: VersionAssets = get_json(&entry.url)?;
                let idx = version.asset_index;
                let mut resp = get(&idx.url)?;
                let body = resp.body_mut().with_config().limit(64 * 1024 * 1024).read_to_vec().map_err(|e| AssetError::Download { url: idx.url.clone(), reason: e.to_string() })?;
                let got: String = Sha1::digest(&body).iter().map(|b| format!("{b:02x}")).collect();
                if got != idx.sha1 {
                    return Err(AssetError::Checksum { url: idx.url });
                }
                let _ = idx.id;
                std::fs::create_dir_all(dir.join("indexes")).map_err(io(&dir))?;
                std::fs::write(&index_path, &body).map_err(io(&index_path))?;
                body
            }
        };
        let file: AssetIndexFile = serde_json::from_slice(&bytes).map_err(|e| AssetError::Archive(e.to_string()))?;
        Ok(AssetStore { objects: file.objects.into_iter().map(|(k, v)| (k, (v.hash, v.size))).collect(), dir })
    }

    pub fn contains(&self, name: &str) -> bool {
        self.objects.contains_key(name)
    }

    /// Names of files that start with `prefix`.
    pub fn names_under(&self, prefix: &str) -> Vec<String> {
        self.objects.keys().filter(|k| k.starts_with(prefix)).cloned().collect()
    }

    /// Whether a file is already in the cache at its full size.
    pub fn is_cached(&self, name: &str) -> bool {
        let Some((hash, size)) = self.objects.get(name) else { return false };
        std::fs::metadata(self.dir.join("objects").join(&hash[..2]).join(hash)).is_ok_and(|m| m.len() == *size)
    }

    /// A file's bytes by its name in the index, such as `minecraft/sounds.json`.
    pub fn read(&self, name: &str) -> Result<Vec<u8>, AssetError> {
        let (hash, size) = self.objects.get(name).ok_or_else(|| AssetError::Archive(format!("no asset named {name}")))?;
        let dir = self.dir.join("objects").join(&hash[..2]);
        let path = dir.join(hash);
        if let Ok(b) = std::fs::read(&path) {
            let got: String = Sha1::digest(&b).iter().map(|x| format!("{x:02x}")).collect();
            if got == *hash {
                return Ok(b);
            }
            tracing::warn!("cached asset {name} failed its checksum; downloading again");
        }
        let url = format!("{OBJECTS_URL}/{}/{hash}", &hash[..2]);
        let mut resp = get(&url)?;
        let body = resp.body_mut().with_config().limit(size + 1024).read_to_vec().map_err(|e| AssetError::Download { url: url.clone(), reason: e.to_string() })?;
        let got: String = Sha1::digest(&body).iter().map(|x| format!("{x:02x}")).collect();
        if got != *hash {
            return Err(AssetError::Checksum { url });
        }
        let io = |p: &Path| {
            let p = p.to_path_buf();
            move |source| AssetError::Io { path: p, source }
        };
        std::fs::create_dir_all(&dir).map_err(io(&dir))?;
        let part = dir.join(format!("{hash}.part"));
        std::fs::write(&part, &body).map_err(io(&part))?;
        std::fs::rename(&part, &path).map_err(io(&path))?;
        Ok(body)
    }
}
