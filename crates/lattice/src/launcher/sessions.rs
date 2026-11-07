//! Game sessions as child processes: starting them, listening to them, stopping them.

use crate::ipc::{Control, LaunchSpec, Phase, Status};
use crossbeam_channel::{Receiver, unbounded};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::time::{Duration, Instant};

const MAX_LOG_LINES: usize = 600;
/// How long a session gets to disconnect cleanly before it's killed.
const STOP_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

pub struct LogLine {
    /// Seconds since the session started.
    pub at: f32,
    pub level: LogLevel,
    pub text: String,
}

enum Message {
    Status(Status),
    Log(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// The player or the launcher closed it.
    Clean,
    /// It stopped on its own with an error.
    Crashed(Option<i32>),
}

pub struct SessionProc {
    pub id: u64,
    pub spec: LaunchSpec,
    /// Normalised server address, for telling which sessions share a server.
    pub address_key: String,
    child: Child,
    stdin: Option<ChildStdin>,
    rx: Receiver<Message>,
    pub status: Option<Status>,
    pub log: VecDeque<LogLine>,
    pub started: Instant,
    pub exit: Option<Exit>,
    pub memory_mb: Option<u64>,
    last_memory: Option<Instant>,
    stopping_since: Option<Instant>,
    /// Set once the session has been in the world.
    pub reached_play: bool,
}

/// The running executable. After a rebuild, Linux reports the old path with
/// " (deleted)"; the file at that path is then the new build, which is what we want.
fn own_exe() -> std::io::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let text = exe.to_string_lossy();
    match text.strip_suffix(" (deleted)") {
        Some(p) => Ok(PathBuf::from(p)),
        None => Ok(exe),
    }
}

fn parse_level(line: &str) -> LogLevel {
    let t = line.trim_start();
    if t.starts_with("ERROR") || t.starts_with("PANIC") || t.starts_with("thread ") {
        LogLevel::Error
    } else if t.starts_with("WARN") {
        LogLevel::Warn
    } else {
        LogLevel::Info
    }
}

impl SessionProc {
    pub fn spawn(id: u64, spec: LaunchSpec, address_key: String) -> std::io::Result<SessionProc> {
        let mut cmd = Command::new(own_exe()?);
        cmd.arg("--session").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        // Development aids aimed at the launcher mustn't reach its children;
        // LATTICE_CHILD_* versions are passed on instead, {n} being the session number.
        for var in ["SHOTS", "SCRIPT", "EXIT_AFTER"] {
            cmd.env_remove(format!("LATTICE_{var}"));
            if let Ok(v) = std::env::var(format!("LATTICE_CHILD_{var}")) {
                cmd.env(format!("LATTICE_{var}"), v.replace("{n}", &id.to_string()));
            }
        }
        cmd.env_remove("LATTICE_AUTOLAUNCH").env_remove("LATTICE_SCREEN");
        let mut child = cmd.spawn()?;
        let mut stdin = child.stdin.take().expect("piped stdin");
        let line = serde_json::to_string(&spec).map_err(std::io::Error::other)?;
        writeln!(stdin, "{line}")?;
        stdin.flush()?;

        let (tx, rx) = unbounded();
        let stdout = child.stdout.take().expect("piped stdout");
        let tx2 = tx.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let msg = match serde_json::from_str::<Status>(&line) {
                    Ok(s) => Message::Status(s),
                    Err(_) => Message::Log(line),
                };
                if tx2.send(msg).is_err() {
                    return;
                }
            }
        });
        let stderr = child.stderr.take().expect("piped stderr");
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if tx.send(Message::Log(line)).is_err() {
                    return;
                }
            }
        });
        Ok(SessionProc {
            id,
            spec,
            address_key,
            child,
            stdin: Some(stdin),
            rx,
            status: None,
            log: VecDeque::new(),
            started: Instant::now(),
            exit: None,
            memory_mb: None,
            last_memory: None,
            stopping_since: None,
            reached_play: false,
        })
    }

    pub fn name(&self) -> &str {
        self.spec.profile.name()
    }

    pub fn running(&self) -> bool {
        self.exit.is_none()
    }

    /// Actually connected (or connecting), as opposed to showing why it ended.
    pub fn on_server(&self) -> bool {
        self.running() && !self.ended()
    }

    /// The connection is over but the window is still open.
    pub fn ended(&self) -> bool {
        self.status.as_ref().is_some_and(|s| s.phase == Phase::Ended)
    }

    pub fn stopping(&self) -> bool {
        self.stopping_since.is_some()
    }

    pub fn send(&mut self, c: Control) {
        if let (Some(stdin), Ok(line)) = (&mut self.stdin, serde_json::to_string(&c)) {
            let _ = writeln!(stdin, "{line}").and_then(|_| stdin.flush());
        }
    }

    /// Asks the session to disconnect and close; killed if it doesn't.
    pub fn stop(&mut self) {
        if self.stopping_since.is_none() {
            self.stopping_since = Some(Instant::now());
            self.send(Control::Stop);
        }
    }

    /// Reads what the session sent. Returns true if anything changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(m) = self.rx.try_recv() {
            changed = true;
            match m {
                Message::Status(s) => {
                    self.reached_play |= s.phase == Phase::Playing;
                    self.status = Some(s);
                }
                Message::Log(text) => {
                    self.log.push_back(LogLine { at: self.started.elapsed().as_secs_f32(), level: parse_level(&text), text });
                    if self.log.len() > MAX_LOG_LINES {
                        self.log.pop_front();
                    }
                }
            }
        }
        if self.exit.is_some() {
            return changed;
        }
        if let Ok(Some(status)) = self.child.try_wait() {
            self.exit = Some(if status.success() { Exit::Clean } else { Exit::Crashed(status.code()) });
            self.memory_mb = None;
            return true;
        }
        if self.stopping_since.is_some_and(|t| t.elapsed() > STOP_TIMEOUT) {
            tracing::warn!("session {} didn't stop in time; killing it", self.id);
            let _ = self.child.kill();
        }
        if self.last_memory.is_none_or(|t| t.elapsed() > Duration::from_secs(1)) {
            self.last_memory = Some(Instant::now());
            self.memory_mb = read_memory_mb(self.child.id());
        }
        changed
    }

    /// The session's state in words.
    pub fn state_text(&self) -> String {
        match self.exit {
            Some(Exit::Clean) => return "Closed".into(),
            Some(Exit::Crashed(Some(code))) => return format!("Crashed with exit code {code}"),
            Some(Exit::Crashed(None)) => return "Crashed (killed by the system)".into(),
            None => {}
        }
        if self.stopping() {
            return "Stopping…".into();
        }
        match &self.status {
            None => "Starting…".into(),
            Some(s) if s.phase == Phase::Playing && !s.focused => "In game, in the background".into(),
            Some(s) if s.detail.is_empty() => "Starting…".into(),
            Some(s) => s.detail.clone(),
        }
    }
}

impl Drop for SessionProc {
    fn drop(&mut self) {
        // Closing the launcher leaves running sessions open. Their pipes close,
        // which they treat as "nobody is listening any more".
        let _ = self.child.try_wait();
    }
}

/// Resident memory of a process. Only available where /proc exists.
fn read_memory_mb(pid: u32) -> Option<u64> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let text = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let line = text.lines().find(|l| l.starts_with("VmRSS:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb / 1024)
}

/// A name for another session on the same server: `base`, then `base2`, `base3`…
/// always within the 16 characters servers accept.
pub fn free_name(base: &str, used: &[String]) -> String {
    let taken = |n: &str| used.iter().any(|u| u.eq_ignore_ascii_case(n));
    if !taken(base) {
        return base.to_string();
    }
    (2u32..)
        .map(|n| {
            let suffix = n.to_string();
            let stem: String = base.chars().take(16 - suffix.len()).collect();
            format!("{stem}{suffix}")
        })
        .find(|c| !taken(c))
        .expect("an unused name exists")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_names() {
        let used = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(free_name("Alex", &used(&[])), "Alex");
        assert_eq!(free_name("Alex", &used(&["alex"])), "Alex2");
        assert_eq!(free_name("Alex", &used(&["Alex", "Alex2"])), "Alex3");
        // Stays within 16 characters.
        let long = "sixteen_chars_xx";
        assert_eq!(long.len(), 16);
        let n = free_name(long, &used(&[long]));
        assert_eq!(n, "sixteen_chars_x2");
        assert!(n.len() <= 16);
    }

    #[test]
    fn log_levels() {
        assert_eq!(parse_level("  WARN lattice: x"), LogLevel::Warn);
        assert_eq!(parse_level(" ERROR x"), LogLevel::Error);
        assert_eq!(parse_level("thread 'main' panicked"), LogLevel::Error);
        assert_eq!(parse_level("  INFO x"), LogLevel::Info);
    }
}
