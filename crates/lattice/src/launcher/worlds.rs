//! Worlds on this computer. The game's world lives in Mojang's server program, so a local world
//! is that program running quietly on this machine with only this machine allowed in.

use crate::config::Paths;
use crossbeam_channel::{Receiver, Sender, unbounded};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct World {
    #[serde(skip)]
    pub folder: PathBuf,
    pub name: String,
    pub game_mode: String,
    pub difficulty: String,
    pub hardcore: bool,
    /// "normal", "flat", "large_biomes" or "amplified".
    pub kind: String,
    pub seed: String,
    pub allow_commands: bool,
    pub created: u64,
    #[serde(default)]
    pub last_played: Option<u64>,
    /// Where it was last served, so it keeps its place in the launcher's memory.
    #[serde(default)]
    pub port: Option<u16>,
}

/// What the "new world" form holds.
#[derive(Debug, Clone, PartialEq)]
pub struct Form {
    pub name: String,
    pub game_mode: usize,
    pub difficulty: usize,
    pub kind: usize,
    pub seed: String,
    pub allow_commands: bool,
    pub eula: bool,
    pub error: Option<String>,
}

impl Default for Form {
    fn default() -> Self {
        Form { name: "New world".into(), game_mode: 0, difficulty: 2, kind: 0, seed: String::new(), allow_commands: false, eula: false, error: None }
    }
}

/// The server's word for each choice, and how it's shown.
pub const GAME_MODES: [(&str, &str, &str); 4] = [
    ("survival", "Survival", "Gather what you need; you can be hurt."),
    ("creative", "Creative", "Everything's available and nothing can hurt you."),
    ("adventure", "Adventure", "Survival, but blocks can't be broken freely."),
    ("hardcore", "Hardcore", "Survival on hard, and dying ends the world."),
];
pub const DIFFICULTIES: [(&str, &str); 4] = [("peaceful", "Peaceful"), ("easy", "Easy"), ("normal", "Normal"), ("hard", "Hard")];
pub const KINDS: [(&str, &str); 4] = [("normal", "Normal"), ("flat", "Flat"), ("large_biomes", "Large biomes"), ("amplified", "Amplified")];

pub fn worlds_dir(paths: &Paths) -> PathBuf {
    paths.data.join("worlds")
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Every world, the one played last first.
pub fn list(paths: &Paths) -> Vec<World> {
    let mut out = Vec::new();
    let Ok(dir) = std::fs::read_dir(worlds_dir(paths)) else { return out };
    for entry in dir.flatten() {
        let folder = entry.path();
        // Worlds made before the rename from Patina keep their old marker file.
        let old = folder.join("patina-world.json");
        if old.exists() {
            let _ = std::fs::rename(&old, folder.join("lattice-world.json"));
        }
        let Ok(text) = std::fs::read_to_string(folder.join("lattice-world.json")) else { continue };
        match serde_json::from_str::<World>(&text) {
            Ok(mut w) => {
                w.folder = folder;
                out.push(w);
            }
            Err(e) => tracing::warn!("couldn't read {}: {e}", folder.display()),
        }
    }
    out.sort_by_key(|w| std::cmp::Reverse(w.last_played.unwrap_or(w.created)));
    out
}

/// A folder name for a world: plain letters, not already taken.
pub fn folder_for(name: &str, parent: &Path) -> PathBuf {
    let base: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    let base = base.trim_matches('-').to_string();
    let base = if base.is_empty() { "world".to_string() } else { base };
    let mut path = parent.join(&base);
    let mut n = 2;
    while path.exists() {
        path = parent.join(format!("{base}-{n}"));
        n += 1;
    }
    path
}

pub fn save(w: &World) -> std::io::Result<()> {
    std::fs::create_dir_all(&w.folder)?;
    let text = serde_json::to_string_pretty(w).map_err(std::io::Error::other)?;
    std::fs::write(w.folder.join("lattice-world.json"), text)
}

pub fn create(paths: &Paths, form: &Form) -> Result<World, String> {
    let name = form.name.trim();
    if name.is_empty() {
        return Err("Give the world a name.".into());
    }
    if !form.eula {
        return Err("Worlds run on Mojang's server program, which needs its license accepted first.".into());
    }
    let (mode, _, _) = GAME_MODES[form.game_mode.min(3)];
    let hardcore = mode == "hardcore";
    let world = World {
        folder: folder_for(name, &worlds_dir(paths)),
        name: name.to_string(),
        game_mode: if hardcore { "survival".into() } else { mode.into() },
        difficulty: if hardcore { "hard".into() } else { DIFFICULTIES[form.difficulty.min(3)].0.into() },
        hardcore,
        kind: KINDS[form.kind.min(3)].0.into(),
        seed: form.seed.trim().to_string(),
        allow_commands: form.allow_commands,
        created: now(),
        last_played: None,
        port: None,
    };
    save(&world).map_err(|e| format!("Couldn't make a folder for the world: {e}"))?;
    Ok(world)
}

/// `server.properties` for a world, keeping it to this machine.
pub fn server_properties(w: &World, port: u16) -> String {
    let level_type = format!("minecraft\\:{}", w.kind);
    // A seed can be typed as anything, but a line break would start a new setting.
    let seed: String = w.seed.chars().filter(|c| !c.is_control()).collect();
    format!(
        "server-ip=127.0.0.1\nserver-port={port}\nonline-mode=false\nenforce-secure-profile=false\nenable-query=false\nenable-rcon=false\nmotd={}\nlevel-name=world\nlevel-seed={seed}\nlevel-type={level_type}\ngamemode={}\nforce-gamemode=false\ndifficulty={}\nhardcore={}\nmax-players=8\nspawn-protection=0\nview-distance=16\nsimulation-distance=10\nwhite-list=false\n",
        w.name.replace(['\n', '\r'], " "),
        w.game_mode,
        w.difficulty,
        w.hardcore,
    )
}

fn free_port(wanted: Option<u16>) -> std::io::Result<u16> {
    if let Some(p) = wanted {
        if std::net::TcpListener::bind(("127.0.0.1", p)).is_ok() {
            return Ok(p);
        }
    }
    Ok(std::net::TcpListener::bind(("127.0.0.1", 0))?.local_addr()?.port())
}

#[derive(Debug, Clone, PartialEq)]
pub enum HostEvent {
    /// What it's doing, and how far along if that's known.
    Step(String, Option<f32>),
    Ready(u16),
    Failed(String),
    Stopped,
}

/// A world being served.
pub struct Host {
    pub world: World,
    pub step: String,
    pub fraction: Option<f32>,
    pub port: Option<u16>,
    pub failed: Option<String>,
    pub stopping: Option<Instant>,
    events: Receiver<HostEvent>,
    process: Sender<Control>,
}

enum Control {
    Stop,
}

impl Host {
    pub fn start(paths: &Paths, world: World, owner: String, memory_mb: u32) -> Host {
        let (tx, events) = unbounded();
        let (process, control) = unbounded();
        let cache = paths.cache.clone();
        let w = world.clone();
        let spawned = std::thread::Builder::new().name("world server".into()).spawn(move || {
            if let Err(e) = serve(&cache, w, owner, memory_mb, &tx, control) {
                let _ = tx.send(HostEvent::Failed(e));
            }
            let _ = tx.send(HostEvent::Stopped);
        });
        if let Err(e) = spawned {
            tracing::error!("couldn't start the world's thread: {e}");
        }
        Host { world, step: "Starting".into(), fraction: None, port: None, failed: None, stopping: None, events, process }
    }

    /// Takes in what's happened. Returns true once it's stopped for good.
    pub fn pump(&mut self) -> Option<HostEvent> {
        let e = self.events.try_recv().ok()?;
        match &e {
            HostEvent::Step(step, f) => {
                self.step = step.clone();
                self.fraction = *f;
            }
            HostEvent::Ready(p) => {
                self.port = Some(*p);
                self.step = "Running".into();
                self.fraction = None;
            }
            HostEvent::Failed(why) => self.failed = Some(why.clone()),
            HostEvent::Stopped => self.port = None,
        }
        Some(e)
    }

    pub fn stop(&mut self) {
        if self.stopping.is_none() {
            self.stopping = Some(Instant::now());
            self.step = "Saving and closing the world".into();
            self.fraction = None;
            let _ = self.process.send(Control::Stop);
        }
    }

    pub fn address(&self) -> Option<String> {
        self.port.map(|p| format!("127.0.0.1:{p}"))
    }
}

fn serve(cache: &Path, mut w: World, owner: String, memory_mb: u32, tx: &Sender<HostEvent>, control: Receiver<Control>) -> Result<(), String> {
    let step = |s: &str, f: Option<f32>| {
        let _ = tx.send(HostEvent::Step(s.to_string(), f));
    };
    step("Getting the server program ready", None);
    let files = lattice_assets::runtime::ensure_server(cache, &|what, p| {
        let f = p.total.map(|t| p.done as f32 / t.max(1) as f32);
        let _ = tx.send(HostEvent::Step(what.to_string(), f));
    })
    .map_err(|e| format!("Couldn't get the server program: {e}"))?;

    let port = free_port(w.port).map_err(|e| format!("Couldn't find a free port: {e}"))?;
    w.port = Some(port);
    w.last_played = Some(now());
    let _ = save(&w);
    let write = |name: &str, text: String| std::fs::write(w.folder.join(name), text).map_err(|e| format!("Couldn't write {name}: {e}"));
    write("server.properties", server_properties(&w, port))?;
    // Only reached with the box ticked when the world was made.
    write("eula.txt", "eula=true\n".to_string())?;

    step("Starting the world", None);
    let mut child = Command::new(&files.java)
        .arg(format!("-Xmx{memory_mb}M"))
        .arg("-jar")
        .arg(&files.jar)
        .arg("--nogui")
        .current_dir(&w.folder)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Couldn't start Java: {e}"))?;
    let result = watch(&mut child, &w, &owner, port, tx, &control);
    // Whatever happened, it mustn't be left running by itself.
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
        }
    }
    result
}

fn watch(child: &mut Child, w: &World, owner: &str, port: u16, tx: &Sender<HostEvent>, control: &Receiver<Control>) -> Result<(), String> {
    let stdout = child.stdout.take().ok_or("the server has no output")?;
    let mut stdin: ChildStdin = child.stdin.take().ok_or("the server takes no input")?;
    let (lines_tx, lines) = unbounded();
    std::thread::Builder::new()
        .name("world output".into())
        .spawn(move || {
            for line in std::io::BufReader::new(stdout).lines().map_while(Result::ok) {
                if lines_tx.send(line).is_err() {
                    return;
                }
            }
        })
        .map_err(|e| e.to_string())?;
    let mut ready = false;
    let started = Instant::now();
    loop {
        crossbeam_channel::select! {
            recv(lines) -> line => match line {
                Ok(line) => {
                    if !ready {
                        // "Preparing spawn area: 40%" and the like say how it's going.
                        if let Some(p) = line.split("Preparing spawn area: ").nth(1).and_then(|r| r.trim_end_matches('%').parse::<f32>().ok()) {
                            let _ = tx.send(HostEvent::Step("Preparing the land".into(), Some(p / 100.0)));
                        }
                        if line.contains("]: Done (") {
                            ready = true;
                            if !w.allow_commands {
                                let _ = writeln!(stdin, "deop {owner}");
                                let _ = stdin.flush();
                            }
                            let _ = tx.send(HostEvent::Ready(port));
                        }
                    }
                    // Someone who's never been here can't be made an operator until they arrive.
                    if w.allow_commands && line.contains(&format!("{owner} joined the game")) {
                        let _ = writeln!(stdin, "op {owner}");
                        let _ = stdin.flush();
                    }
                    if line.contains("FAILED TO BIND TO PORT") {
                        return Err("Another program took the port this world needs. Try again.".into());
                    }
                }
                // The program's gone.
                Err(_) => {
                    return if ready {
                        Ok(())
                    } else {
                        Err(format!("The server program stopped by itself. Its log is in {}.", w.folder.join("logs").display()))
                    };
                }
            },
            recv(control) -> message => match message {
                Ok(Control::Stop) | Err(_) => {
                    let _ = stdin.write_all(b"stop\n");
                    let _ = stdin.flush();
                    // Keep reading until it finishes, without asking twice.
                    for _ in lines.iter() {}
                    return Ok(());
                }
            },
            default(Duration::from_secs(1)) => {
                if !ready && started.elapsed() > Duration::from_secs(600) {
                    let _ = child.kill();
                    return Err("The world took over ten minutes to start, so it was stopped.".into());
                }
            }
        }
    }
}

/// `lattice --host-world`: serves a world, making it first if it's new.
pub fn host_from_command_line(paths: &Paths, name: &str, player: &str, kind: &str, accept: bool, memory_mb: u32) -> anyhow::Result<()> {
    let world = match list(paths).into_iter().find(|w| w.name == name) {
        Some(w) => w,
        None => {
            let form = Form {
                name: name.to_string(),
                kind: KINDS.iter().position(|k| k.0 == kind).unwrap_or(0),
                eula: accept,
                allow_commands: true,
                ..Default::default()
            };
            create(paths, &form).map_err(|e| anyhow::anyhow!(e))?
        }
    };
    let mut host = Host::start(paths, world, player.to_string(), memory_mb);
    // Input ending, as when whoever started this goes away, closes the world.
    let (done_tx, done) = unbounded::<()>();
    std::thread::spawn(move || {
        let mut sink = String::new();
        while std::io::stdin().read_line(&mut sink).is_ok_and(|n| n > 0) {
            sink.clear();
        }
        let _ = done_tx.send(());
    });
    loop {
        if done.try_recv().is_ok() {
            host.stop();
        }
        match host.pump() {
            Some(HostEvent::Ready(port)) => println!("ready 127.0.0.1:{port}"),
            Some(HostEvent::Step(step, f)) => {
                let percent = f.map_or(String::new(), |f| format!(" {:.0}%", f * 100.0));
                println!("step {step}{percent}");
            }
            Some(HostEvent::Failed(why)) => anyhow::bail!(why),
            Some(HostEvent::Stopped) => return Ok(()),
            None => std::thread::sleep(Duration::from_millis(50)),
        }
        let _ = std::io::stdout().flush();
    }
}

/// A short "when" for a list.
pub fn ago(when: u64) -> String {
    let secs = now().saturating_sub(when);
    match secs {
        0..=89 => "just now".into(),
        90..=3599 => format!("{} minutes ago", secs / 60),
        3600..=7199 => "an hour ago".into(),
        7200..=86399 => format!("{} hours ago", secs / 3600),
        86400..=172799 => "yesterday".into(),
        _ => format!("{} days ago", secs / 86400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> World {
        World {
            folder: PathBuf::new(),
            name: "My world".into(),
            game_mode: "creative".into(),
            difficulty: "easy".into(),
            hardcore: false,
            kind: "flat".into(),
            seed: "abc\nserver-ip=0.0.0.0".into(),
            allow_commands: true,
            created: 0,
            last_played: None,
            port: None,
        }
    }

    #[test]
    fn stays_on_this_machine() {
        let p = server_properties(&sample(), 25601);
        assert!(p.contains("server-ip=127.0.0.1\n"));
        assert!(p.contains("server-port=25601\n"));
        assert!(p.contains("level-type=minecraft\\:flat\n"));
        // A seed can't smuggle in a setting.
        assert!(p.contains("level-seed=abcserver-ip=0.0.0.0\n"));
        assert_eq!(p.lines().filter(|l| l.starts_with("server-ip=")).count(), 1);
    }

    #[test]
    fn folder_names_are_plain_and_unique() {
        let dir = std::env::temp_dir().join(format!("lattice-worlds-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("my-world")).unwrap();
        assert_eq!(folder_for("../My World!", &dir).file_name().unwrap(), "my-world-2");
        assert_eq!(folder_for("???", &dir).file_name().unwrap(), "world");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
