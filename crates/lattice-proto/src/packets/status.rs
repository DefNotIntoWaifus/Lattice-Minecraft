use super::{Encoded, enc};
use crate::codec::{DResult, Reader};
use crate::ids::status::{cb, sb};

pub fn status_request() -> Encoded {
    enc(sb::STATUS_REQUEST, |_| {})
}

pub fn ping_request(payload: i64) -> Encoded {
    enc(sb::PING_REQUEST, |w| {
        w.i64(payload);
    })
}

pub enum Cb {
    StatusResponse(String),
    PongResponse(i64),
    Unknown(i32),
}

pub fn decode(id: i32, r: &mut Reader) -> DResult<Cb> {
    Ok(match id {
        cb::STATUS_RESPONSE => Cb::StatusResponse(r.string(32767)?),
        cb::PONG_RESPONSE => Cb::PongResponse(r.i64()?),
        other => Cb::Unknown(other),
    })
}
