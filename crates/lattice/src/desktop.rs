//! What makes Lattice behave as an app on the desktop instead of a program run
//! from a terminal: its windows say which app they belong to, a second launch
//! points at the launcher that's already open, and a failure to start says so.

use std::fs::File;
use std::path::Path;
use winit::event_loop::ActiveEventLoop;
use winit::platform::startup_notify::{EventLoopExtStartupNotify, WindowAttributesExtStartupNotify, reset_activation_token_env};
use winit::window::{Icon, WindowAttributes};

/// Has to match the installed `lattice.desktop` (see tools/install.sh): desktops
/// use it to find the name and icon to show for a window.
pub const APP_ID: &str = "lattice";

const ICON_PNG: &[u8] = include_bytes!("../assets/icon/lattice-256.png");

fn icon() -> Option<Icon> {
    let image = image::load_from_memory(ICON_PNG).ok()?.to_rgba8();
    let (width, height) = image.dimensions();
    Icon::from_rgba(image.into_raw(), width, height).ok()
}

/// Names the app on a new window. Wayland takes the icon from the desktop file
/// this points to; X11 and Windows use the one given here.
pub fn identify(attrs: WindowAttributes) -> WindowAttributes {
    #[cfg(target_os = "linux")]
    let attrs = winit::platform::wayland::WindowAttributesExtWayland::with_name(attrs, APP_ID, APP_ID);
    attrs.with_window_icon(icon())
}

/// Uses the token the desktop gave this launch, so the window opens in front and
/// the desktop stops showing it as starting. Game windows must not reuse it.
pub fn use_launch_token(event_loop: &ActiveEventLoop, attrs: WindowAttributes) -> WindowAttributes {
    let token = event_loop.read_token_from_env();
    reset_activation_token_env();
    match token {
        Some(token) => attrs.with_activation_token(token),
        None => attrs,
    }
}

/// Being the one launcher for these settings. The lock lasts as long as the
/// process does, however it ends, so a crash can't block the next start.
pub struct Instance {
    _lock: Option<File>,
    #[cfg(unix)]
    listener: Option<std::os::unix::net::UnixListener>,
}

pub enum Claim {
    First(Instance),
    AlreadyOpen,
}

/// Decides whether this launch becomes the launcher, or tells the one already
/// open about it. If the lock can't be used, opening a second launcher is the
/// lesser problem: this never stops the app from starting.
pub fn claim_launcher(dir: &Path) -> Claim {
    let lock = match File::options().create(true).truncate(false).write(true).open(dir.join("launcher.lock")) {
        Ok(file) => file,
        Err(e) => {
            eprintln!("Couldn't check for an open launcher: {e}");
            return Claim::First(Instance { _lock: None, #[cfg(unix)] listener: None });
        }
    };
    match lock.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => {
            #[cfg(unix)]
            tell_open_launcher(&dir.join("launcher.sock"));
            return Claim::AlreadyOpen;
        }
        Err(std::fs::TryLockError::Error(e)) => eprintln!("Couldn't check for an open launcher: {e}"),
    }
    #[cfg(unix)]
    let listener = {
        let path = dir.join("launcher.sock");
        let _ = std::fs::remove_file(&path);
        match std::os::unix::net::UnixListener::bind(&path) {
            Ok(listener) => Some(listener),
            Err(e) => {
                eprintln!("Another launch won't be able to find this launcher: {e}");
                None
            }
        }
    };
    Claim::First(Instance { _lock: Some(lock), #[cfg(unix)] listener })
}

#[cfg(unix)]
fn tell_open_launcher(path: &Path) {
    // Two quick clicks: the other launch may hold the lock but not be listening yet.
    for _ in 0..20 {
        if std::os::unix::net::UnixStream::connect(path).is_ok() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

impl Instance {
    /// Calls `raise` each time someone launches the app again.
    pub fn on_second_launch(&mut self, raise: impl Fn() + Send + 'static) {
        #[cfg(unix)]
        if let Some(listener) = self.listener.take() {
            let spawned = std::thread::Builder::new().name("second-launch".into()).spawn(move || {
                for connection in listener.incoming() {
                    if connection.is_ok() {
                        raise();
                    }
                }
            });
            if let Err(e) = spawned {
                tracing::warn!("couldn't listen for a second launch: {e}");
            }
        }
        #[cfg(not(unix))]
        let _ = raise;
    }
}

/// Started from a menu there's no terminal to print to, so a failure to start
/// would look like nothing happening. There's no window of our own to use either.
pub fn report_start_failure(error: &str, log: &Path) {
    use std::io::IsTerminal;
    tracing::error!("couldn't start: {error}");
    eprintln!("Lattice couldn't start: {error}");
    if std::io::stderr().is_terminal() {
        return;
    }
    // The windowing library puts its own source file ahead of the reason.
    let error = match error.strip_prefix("os error at ").and_then(|rest| rest.split_once(": ")) {
        Some((_, reason)) => reason,
        None => error,
    };
    let text = format!("Lattice couldn't start.\n\n{error}\n\nThe log is at {}", log.display());
    let shown = |program: &str, args: &[&str]| std::process::Command::new(program).args(args).status().is_ok();
    let _ = shown("kdialog", &["--title", "Lattice", "--error", &text])
        || shown("zenity", &["--error", "--no-markup", "--title=Lattice", &format!("--text={text}")])
        || shown("notify-send", &["--app-name=Lattice", "--urgency=critical", "Lattice couldn't start", error]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icon_built_in_can_be_read() {
        assert!(icon().is_some());
    }

    #[cfg(unix)]
    #[test]
    fn a_second_launch_finds_the_first_and_can_follow_a_closed_one() {
        let dir = std::env::temp_dir().join(format!("lattice-instance-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let Claim::First(mut first) = claim_launcher(&dir) else { panic!("nothing else is open") };
        let (told, heard) = std::sync::mpsc::channel();
        first.on_second_launch(move || told.send(()).unwrap());

        assert!(matches!(claim_launcher(&dir), Claim::AlreadyOpen));
        heard.recv_timeout(std::time::Duration::from_secs(5)).expect("the open launcher should hear about it");

        // Ending releases the lock, and the leftover socket doesn't get in the way.
        drop(first);
        assert!(matches!(claim_launcher(&dir), Claim::First(_)));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
