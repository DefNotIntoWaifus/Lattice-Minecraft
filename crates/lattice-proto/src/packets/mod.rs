//! Packet definitions, one module per connection state.
//!
//! Clientbound packets are decoded into enums; packets the client doesn't use
//! yet decode to `Unknown(id)` and are skipped. Serverbound packets are enums
//! that encode themselves. Field layouts follow the protocol documentation for
//! 26.3 (protocol 777); IDs come from the generated `ids` module.

pub mod config;
pub mod handshake;
pub mod login;
pub mod play;
pub mod status;

use crate::codec::{DResult, Reader, Writer};
use uuid::Uuid;

/// `Game Profile` data type.
#[derive(Debug, Clone, Default)]
pub struct GameProfile {
    pub uuid: Uuid,
    pub name: String,
    pub properties: Vec<ProfileProperty>,
}

#[derive(Debug, Clone)]
pub struct ProfileProperty {
    pub name: String,
    pub value: String,
    pub signature: Option<String>,
}

pub(crate) fn read_properties(r: &mut Reader) -> DResult<Vec<ProfileProperty>> {
    r.array(|r| {
        Ok(ProfileProperty { name: r.string(64)?, value: r.string(32767)?, signature: r.option(|r| r.string(1024))? })
    })
}

impl GameProfile {
    pub(crate) fn read(r: &mut Reader) -> DResult<Self> {
        Ok(Self { uuid: r.uuid()?, name: r.string(16)?, properties: read_properties(r)? })
    }
}

/// Encoded serverbound packet: ID + body.
pub struct Encoded {
    pub id: i32,
    pub body: Vec<u8>,
}

pub(crate) fn enc(id: i32, f: impl FnOnce(&mut Writer)) -> Encoded {
    let mut w = Writer::new();
    f(&mut w);
    Encoded { id, body: w.buf }
}
