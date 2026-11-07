use super::{Encoded, GameProfile, enc};
use crate::codec::{DResult, Reader};
use crate::ids::login::{cb, sb};
use crate::text::Text;
use uuid::Uuid;

#[derive(Debug)]
pub enum Cb {
    Disconnect(Text),
    EncryptionRequest { server_id: String, public_key: Vec<u8>, verify_token: Vec<u8>, should_authenticate: bool },
    LoginSuccess { profile: GameProfile, session_id: Uuid },
    SetCompression(i32),
    PluginRequest { message_id: i32, channel: String },
    CookieRequest(String),
    Unknown(i32),
}

pub fn decode(id: i32, r: &mut Reader) -> DResult<Cb> {
    Ok(match id {
        cb::LOGIN_DISCONNECT => Cb::Disconnect(r.json_text()?),
        cb::HELLO => Cb::EncryptionRequest {
            server_id: r.string(20)?,
            public_key: r.byte_array()?,
            verify_token: r.byte_array()?,
            should_authenticate: r.bool()?,
        },
        cb::LOGIN_FINISHED => Cb::LoginSuccess { profile: GameProfile::read(r)?, session_id: r.uuid()? },
        cb::LOGIN_COMPRESSION => Cb::SetCompression(r.var_i32()?),
        cb::CUSTOM_QUERY => {
            let message_id = r.var_i32()?;
            let channel = r.identifier()?;
            r.rest();
            Cb::PluginRequest { message_id, channel }
        }
        cb::COOKIE_REQUEST => Cb::CookieRequest(r.identifier()?),
        other => Cb::Unknown(other),
    })
}

pub fn hello(name: &str, uuid: Uuid) -> Encoded {
    enc(sb::HELLO, |w| {
        w.string(name).uuid(uuid);
    })
}

pub fn encryption_response(shared_secret: &[u8], verify_token: &[u8]) -> Encoded {
    enc(sb::KEY, |w| {
        w.byte_array(shared_secret).byte_array(verify_token);
    })
}

/// The vanilla client never understands login plugin requests.
pub fn plugin_response_not_understood(message_id: i32) -> Encoded {
    enc(sb::CUSTOM_QUERY_ANSWER, |w| {
        w.var_i32(message_id).bool(false);
    })
}

pub fn login_acknowledged() -> Encoded {
    enc(sb::LOGIN_ACKNOWLEDGED, |_| {})
}

/// We never store cookies, so every request gets "no cookie".
pub fn cookie_response_empty(key: &str) -> Encoded {
    enc(sb::COOKIE_RESPONSE, |w| {
        w.string(key).bool(false);
    })
}
