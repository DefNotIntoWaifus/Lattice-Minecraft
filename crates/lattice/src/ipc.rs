//! What the launcher and a game window say to each other.
//!
//! Every game session is its own process, so one crash can't take down the
//! launcher or the other sessions. The launcher starts `lattice --session` and
//! writes one line of JSON to its stdin: the [`LaunchSpec`]. Tokens travel this
//! way rather than as arguments because command lines are visible to every
//! process on the machine. After that, stdin carries [`Control`] lines and
//! stdout carries [`Status`] lines. Logs go to stderr.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Deliberately not `Debug`: it can hold an access token, and must never reach a log.
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SpecProfile {
    Offline { name: String },
    Online { name: String, uuid: Uuid, access_token: String },
}

impl SpecProfile {
    pub fn name(&self) -> &str {
        match self {
            SpecProfile::Offline { name } | SpecProfile::Online { name, .. } => name,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct LaunchSpec {
    pub server_name: String,
    pub address: String,
    pub profile: SpecProfile,
    /// A launcher is reading our stdout.
    pub reports: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Loading,
    Connecting,
    Terrain,
    Playing,
    /// The connection is over but the window is still open, showing why.
    Ended,
}

/// Sent a few times a second. Everything here is measured, not estimated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Status {
    pub phase: Phase,
    /// What's happening, in words: "Logging in…", or why the session ended.
    pub detail: String,
    pub fps: f32,
    pub frame_ms: f32,
    pub worst_ms: f32,
    pub focused: bool,
    pub ping: Option<i32>,
    pub players: Option<usize>,
    pub pos: Option<[f64; 3]>,
    pub dimension: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Control {
    /// Disconnect cleanly and close the window.
    Stop,
    /// The settings file changed; read it again and apply what can be applied live.
    ReloadConfig,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_round_trips_as_one_line() {
        let spec = LaunchSpec {
            server_name: "Local".into(),
            address: "localhost:25566".into(),
            profile: SpecProfile::Online { name: "A".into(), uuid: Uuid::nil(), access_token: "tok".into() },
            reports: true,
        };
        let line = serde_json::to_string(&spec).unwrap();
        assert!(!line.contains('\n'));
        let back: LaunchSpec = serde_json::from_str(&line).unwrap();
        assert_eq!(back.profile.name(), "A");
        assert_eq!(back.address, "localhost:25566");
    }

    #[test]
    fn control_lines() {
        assert_eq!(serde_json::to_string(&Control::Stop).unwrap(), "\"stop\"");
        assert!(matches!(serde_json::from_str::<Control>("\"reload_config\""), Ok(Control::ReloadConfig)));
    }
}
