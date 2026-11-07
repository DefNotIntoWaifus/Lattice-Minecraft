//! Resource packs: the player's own, from a folder, and ones a server offers.
//! A pack only ever replaces pictures, sounds and text in memory. Nothing in it
//! runs, and server packs come over the web, so they're size-limited and checked
//! against the hash the server gave.

use crate::config::Paths;
use crossbeam_channel::{Receiver, Sender, unbounded};
use sha1::{Digest, Sha1};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Bigger than any pack a server should send.
const MAX_PACK_BYTES: u64 = 250 * 1024 * 1024;

pub const STATUS_LOADED: i32 = 0;
pub const STATUS_DECLINED: i32 = 1;
pub const STATUS_FAILED_DOWNLOAD: i32 = 2;
pub const STATUS_ACCEPTED: i32 = 3;
pub const STATUS_DOWNLOADED: i32 = 4;
pub const STATUS_INVALID_URL: i32 = 5;
pub const STATUS_DISCARDED: i32 = 7;

/// Where the player's own packs live.
pub fn user_folder(paths: &Paths) -> PathBuf {
    paths.data.join("resourcepacks")
}

/// The packs in the player's folder, by file or folder name.
pub fn user_packs(paths: &Paths) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(dir) = std::fs::read_dir(user_folder(paths)) {
        for e in dir.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let path = e.path();
            let is_pack = path.join("pack.mcmeta").exists() || name.to_lowercase().ends_with(".zip");
            if is_pack {
                out.push(name);
            }
        }
    }
    out.sort();
    out
}

fn sha1_hex(bytes: &[u8]) -> String {
    Sha1::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Why a server's pack couldn't be fetched, and what to tell the server.
#[derive(Debug)]
pub struct PackError {
    pub status: i32,
    pub message: String,
}

fn download(url: &str, hash: &str, dir: &Path) -> Result<PathBuf, PackError> {
    let failed = |message: String| PackError { status: STATUS_FAILED_DOWNLOAD, message };
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(PackError { status: STATUS_INVALID_URL, message: "its address isn't a web link".into() });
    }
    let hash = hash.trim().to_lowercase();
    let known = hash.len() == 40 && hash.chars().all(|c| c.is_ascii_hexdigit());
    let name = if known { hash.clone() } else { sha1_hex(url.as_bytes()) };
    let path = dir.join(format!("{name}.zip"));
    // Downloaded before, for this server or another.
    if known {
        if let Ok(bytes) = std::fs::read(&path) {
            if sha1_hex(&bytes) == hash {
                return Ok(path);
            }
        }
    }
    std::fs::create_dir_all(dir).map_err(|e| failed(e.to_string()))?;
    let mut response = ureq::get(url)
        .header("User-Agent", concat!("lattice/", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(|e| failed(e.to_string()))?;
    let bytes = response.body_mut().with_config().limit(MAX_PACK_BYTES).read_to_vec().map_err(|e| failed(e.to_string()))?;
    if known && sha1_hex(&bytes) != hash {
        return Err(failed("it didn't match the checksum the server gave".into()));
    }
    crate::config::write_atomic(&path, &bytes, false).map_err(|e| failed(e.to_string()))?;
    Ok(path)
}

struct Fetched {
    id: Uuid,
    result: Result<PathBuf, PackError>,
}

/// The packs the current server has offered, in the order it offered them.
pub struct ServerPacks {
    /// Fetched and in use, or about to be once the next reload finishes.
    pub active: Vec<(Uuid, PathBuf)>,
    /// Told the server "downloaded", still owed "loaded".
    pub awaiting_load: Vec<Uuid>,
    /// Being fetched.
    pub fetching: usize,
    tx: Sender<Fetched>,
    rx: Receiver<Fetched>,
}

impl Default for ServerPacks {
    fn default() -> Self {
        let (tx, rx) = unbounded();
        ServerPacks { active: Vec::new(), awaiting_load: Vec::new(), fetching: 0, tx, rx }
    }
}

impl ServerPacks {
    /// Starts fetching a pack the server offered.
    pub fn fetch(&mut self, id: Uuid, url: String, hash: String, cache: &Path) {
        self.fetching += 1;
        let (tx, dir) = (self.tx.clone(), cache.join("server-packs"));
        let spawned = std::thread::Builder::new().name("pack".into()).spawn(move || {
            let _ = tx.send(Fetched { id, result: download(&url, &hash, &dir) });
        });
        if let Err(e) = spawned {
            tracing::warn!("couldn't start fetching a resource pack: {e}");
            self.fetching -= 1;
        }
    }

    /// Fetches that have finished: each pack's id and how it went.
    pub fn finished(&mut self) -> Vec<(Uuid, Result<PathBuf, PackError>)> {
        let mut out = Vec::new();
        while let Ok(f) = self.rx.try_recv() {
            self.fetching = self.fetching.saturating_sub(1);
            out.push((f.id, f.result));
        }
        out
    }
}

/// Every pack that should be layered, lowest first: the player's own, then the server's.
pub fn layers(paths: &Paths, enabled: &[String], server: &ServerPacks) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = enabled.iter().map(|n| user_folder(paths).join(n)).filter(|p| p.exists()).collect();
    out.extend(server.active.iter().map(|(_, p)| p.clone()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_anything_but_web_links() {
        let dir = std::env::temp_dir().join(format!("lattice-packs-{}", std::process::id()));
        let e = download("file:///etc/passwd", "", &dir).unwrap_err();
        assert_eq!(e.status, STATUS_INVALID_URL);
    }

    #[test]
    fn checksums_read_as_lower_case_hex() {
        assert_eq!(sha1_hex(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
    }
}
