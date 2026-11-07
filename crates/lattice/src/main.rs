//! Lattice: a from-scratch client for Minecraft: Java Edition servers.
//!
//! `lattice` opens the launcher. The launcher starts each game window as
//! `lattice --session` (see ipc.rs). `lattice --join` opens a game window directly.

mod a11y;
mod app;
mod assets;
mod audio;
mod bodies;
mod config;
mod desktop;
mod dev;
mod game;
mod input;
mod ipc;
mod packs;
mod launcher;
mod skins;
mod ui;

use anyhow::Context;
use ipc::{LaunchSpec, SpecProfile};
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

const HELP: &str = "\
Lattice, a client for Minecraft: Java Edition servers

  lattice                         open the launcher
  lattice --join ADDRESS [--name NAME]
                                 open one game window on a server, with an
                                 offline name (for offline-mode servers)
  lattice --host-world NAME       serve a world on this computer with no window;
         [--accept-minecraft-license] [--world-type TYPE] [--name PLAYER]
                                 prints its address, and closes it when input ends
  lattice --help                  show this
";

enum Mode {
    Launcher,
    /// Started by the launcher; the spec arrives on stdin.
    Session,
    Join { address: String, name: Option<String> },
    /// Serves a world with no window, for scripts and tests: prints its port, and closes it
    /// when its input ends.
    HostWorld { name: String, player: String, kind: String, accept_license: bool },
    Help,
}

fn parse_args(args: &[String]) -> Result<Mode, String> {
    let mut mode = Mode::Launcher;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--help" | "-h" => return Ok(Mode::Help),
            "--session" => mode = Mode::Session,
            "--join" => {
                let address = it.next().ok_or("--join needs a server address")?.clone();
                mode = Mode::Join { address, name: None };
            }
            "--host-world" => {
                let name = it.next().ok_or("--host-world needs a world's name")?.clone();
                mode = Mode::HostWorld { name, player: "Player".into(), kind: "normal".into(), accept_license: false };
            }
            "--accept-minecraft-license" => match &mut mode {
                Mode::HostWorld { accept_license, .. } => *accept_license = true,
                _ => return Err("--accept-minecraft-license goes after --host-world NAME".into()),
            },
            "--world-type" => {
                let k = it.next().ok_or("--world-type needs normal, flat, large_biomes or amplified")?.clone();
                match &mut mode {
                    Mode::HostWorld { kind, .. } => *kind = k,
                    _ => return Err("--world-type goes after --host-world NAME".into()),
                }
            }
            "--name" => {
                let n = it.next().ok_or("--name needs a player name")?.clone();
                match &mut mode {
                    Mode::Join { name, .. } => *name = Some(n),
                    Mode::HostWorld { player, .. } => *player = n,
                    _ => return Err("--name goes after --join ADDRESS".into()),
                }
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(mode)
}

/// Log file of a session, named after who is playing so a person can find it.
pub fn session_log_path(paths: &config::Paths, name: &str) -> PathBuf {
    paths.data.join("logs").join(format!("session-{name}.log"))
}

fn init_logging(file: &Path) {
    use tracing_subscriber::prelude::*;
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if file.exists() {
        let _ = std::fs::rename(file, file.with_extension("previous.log"));
    }
    let log_file = std::fs::File::create(file).ok().map(std::sync::Mutex::new);
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,wgpu_core=warn,wgpu_hal=warn,naga=warn,calloop=error,arboard=error,egui_winit::clipboard=error".into());
    // When piped, stderr is the launcher's log view, which adds its own times.
    let piped = !std::io::stderr().is_terminal();
    let timed = (!piped).then(|| tracing_subscriber::fmt::layer().with_writer(std::io::stderr));
    let untimed = piped.then(|| tracing_subscriber::fmt::layer().with_writer(std::io::stderr).with_ansi(false).without_time());
    let registry = tracing_subscriber::registry().with(filter).with(timed).with(untimed);
    match log_file {
        Some(f) => registry.with(tracing_subscriber::fmt::layer().with_ansi(false).with_writer(f)).init(),
        None => registry.init(),
    }
    let default_hook = std::panic::take_hook();
    let path = file.to_path_buf();
    std::panic::set_hook(Box::new(move |info| {
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            let _ = writeln!(f, "PANIC: {info}\n{}", std::backtrace::Backtrace::force_capture());
        }
        default_hook(info);
    }));
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = match parse_args(&args) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{e}\n\n{HELP}");
            std::process::exit(2);
        }
    };
    if matches!(mode, Mode::Help) {
        print!("{HELP}");
        return Ok(());
    }
    let paths = config::Paths::detect();
    paths.ensure();
    let config = config::Config::load(&paths.config);
    if let Mode::HostWorld { name, player, kind, accept_license } = &mode {
        return launcher::worlds::host_from_command_line(&paths, name, player, kind, *accept_license, config.worlds.memory_mb);
    }
    let spec = match mode {
        Mode::Launcher | Mode::Help | Mode::HostWorld { .. } => None,
        Mode::Session => {
            let mut line = String::new();
            std::io::stdin().lock().read_line(&mut line).context("reading the launch request")?;
            Some(serde_json::from_str::<LaunchSpec>(&line).context("the launch request is malformed")?)
        }
        Mode::Join { address, name } => {
            let name = name.unwrap_or_else(|| config.offline_name());
            if !config::valid_player_name(&name) {
                anyhow::bail!("\"{name}\" isn't a valid player name: use 3 to 16 letters, digits or underscores");
            }
            Some(LaunchSpec { server_name: address.clone(), address, profile: SpecProfile::Offline { name }, reports: false })
        }
    };
    match spec {
        None => {
            // Before the logs, which a second launch mustn't start over.
            let desktop::Claim::First(instance) = desktop::claim_launcher(&paths.cache) else {
                println!("Lattice is already open.");
                return Ok(());
            };
            let log = paths.data.join("logs").join("launcher.log");
            init_logging(&log);
            tracing::info!("Lattice {} launcher; settings in {}", env!("CARGO_PKG_VERSION"), paths.config.display());
            let result = launcher::run(paths, config, instance);
            if let Err(e) = &result {
                desktop::report_start_failure(&format!("{e:#}"), &log);
            }
            result
        }
        Some(spec) => {
            init_logging(&session_log_path(&paths, spec.profile.name()));
            tracing::info!("Lattice {}: {} joining {}", env!("CARGO_PKG_VERSION"), spec.profile.name(), spec.address);
            match dev::headless_size() {
                Some((w, h)) => app::run_headless(paths, config, spec, w, h),
                None => app::run(paths, config, spec),
            }
        }
    }
}
