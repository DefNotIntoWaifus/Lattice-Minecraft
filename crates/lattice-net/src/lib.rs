//! Talking to game servers.

pub mod address;
pub mod conn;
pub mod status;

pub use address::ServerAddress;
pub use conn::{ConnectOptions, Connection, Disconnect, Event, Profile, Stage};
pub use status::{ServerStatus, ping};
