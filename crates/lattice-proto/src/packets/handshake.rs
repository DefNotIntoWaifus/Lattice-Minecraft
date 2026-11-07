use super::{Encoded, enc};
use crate::ids;

pub const PROTOCOL_VERSION: i32 = 777;
pub const GAME_VERSION: &str = "26.3";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    Status = 1,
    Login = 2,
    Transfer = 3,
}

pub fn intention(host: &str, port: u16, intent: Intent) -> Encoded {
    enc(ids::handshake::sb::INTENTION, |w| {
        w.var_i32(PROTOCOL_VERSION).string(host).u16(port).var_i32(intent as i32);
    })
}
