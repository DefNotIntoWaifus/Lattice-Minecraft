//! Server list ping: handshake with intent "status", read the JSON status,
//! then measure round-trip time with a ping.

use crate::address::{ServerAddress, resolve};
use base64::Engine;
use lattice_proto::frame::{FrameReader, FrameWriter};
use lattice_proto::packets::{handshake, status};
use lattice_proto::{Reader, Text};
use std::net::TcpStream;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct ServerStatus {
    pub version_name: String,
    pub protocol: i32,
    pub players_online: Option<i64>,
    pub players_max: Option<i64>,
    pub player_sample: Vec<String>,
    pub description: Text,
    /// PNG bytes of the 64×64 server icon.
    pub favicon: Option<Vec<u8>>,
    pub ping_ms: u32,
    pub enforces_secure_chat: bool,
}

impl ServerStatus {
    pub fn compatible(&self) -> bool {
        self.protocol == lattice_proto::PROTOCOL_VERSION
    }
}

pub fn ping(address: &ServerAddress, timeout: Duration) -> Result<ServerStatus, String> {
    let resolved = resolve(address)?;
    let unreachable = || format!("Couldn't reach {}. The server is offline or the address is wrong.", address.display());
    let stream = resolved
        .addrs
        .iter()
        .find_map(|a| TcpStream::connect_timeout(a, timeout).ok())
        .ok_or_else(unreachable)?;
    stream.set_read_timeout(Some(timeout)).ok();
    stream.set_write_timeout(Some(timeout)).ok();
    stream.set_nodelay(true).ok();
    let mut w = FrameWriter::new(stream.try_clone().map_err(|e| e.to_string())?);
    let mut r = FrameReader::new(std::io::BufReader::new(stream));
    let io = |e: std::io::Error| match e.kind() {
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => {
            format!("{} didn't answer in time.", address.display())
        }
        _ => format!("Lost the connection to {} while asking for its status.", address.display()),
    };
    let send = |w: &mut FrameWriter<TcpStream>, p: lattice_proto::packets::Encoded| w.write_packet(p.id, &p.body);
    send(&mut w, handshake::intention(&resolved.handshake_host, resolved.handshake_port, handshake::Intent::Status)).map_err(io)?;
    send(&mut w, status::status_request()).map_err(io)?;

    let json = loop {
        let frame = r.read_frame().map_err(io)?;
        let mut rd = Reader::new(&frame);
        let id = rd.var_i32().map_err(|e| e.to_string())?;
        if let status::Cb::StatusResponse(s) = status::decode(id, &mut rd).map_err(|e| e.to_string())? {
            break s;
        }
    };
    let start = Instant::now();
    let token = start.elapsed().as_nanos() as i64 ^ 0x5041_5449_4E41;
    let ping_ms = match send(&mut w, status::ping_request(token)) {
        Ok(()) => loop {
            match r.read_frame() {
                Ok(frame) => {
                    let mut rd = Reader::new(&frame);
                    if let Ok(id) = rd.var_i32() {
                        if let Ok(status::Cb::PongResponse(_)) = status::decode(id, &mut rd) {
                            break start.elapsed().as_millis() as u32;
                        }
                    }
                }
                // Some proxies close instead of answering the ping; the status is still valid.
                Err(_) => break start.elapsed().as_millis() as u32,
            }
        },
        Err(_) => 0,
    };
    parse_status(&json, ping_ms)
}

pub fn parse_status(json: &str, ping_ms: u32) -> Result<ServerStatus, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|_| "The server sent a status Lattice couldn't read.".to_string())?;
    let version = &v["version"];
    let players = &v["players"];
    let favicon = v["favicon"]
        .as_str()
        .and_then(|s| s.strip_prefix("data:image/png;base64,"))
        .and_then(|b| base64::engine::general_purpose::STANDARD.decode(b.trim()).ok());
    Ok(ServerStatus {
        version_name: version["name"].as_str().unwrap_or("Old").to_string(),
        protocol: version["protocol"].as_i64().unwrap_or(-1) as i32,
        players_online: players["online"].as_i64(),
        players_max: players["max"].as_i64(),
        player_sample: players["sample"]
            .as_array()
            .map(|a| a.iter().filter_map(|p| p["name"].as_str().map(str::to_owned)).collect())
            .unwrap_or_default(),
        description: v.get("description").map(Text::from_json).unwrap_or_else(|| Text::literal("")),
        favicon,
        ping_ms,
        enforces_secure_chat: v["enforcesSecureChat"].as_bool().unwrap_or(false),
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn documented_example() {
        let s = super::parse_status(
            r#"{"version":{"name":"1.21.8","protocol":772},"players":{"max":20,"online":1,"sample":[{"name":"thinkofdeath","id":"4566e69f-c907-48ee-8d71-d7ba5aa00d20"}]},"description":{"text":"Hello, world!"},"enforcesSecureChat":false}"#,
            12,
        )
        .unwrap();
        assert_eq!(s.version_name, "1.21.8");
        assert_eq!(s.players_online, Some(1));
        assert_eq!(s.player_sample, vec!["thinkofdeath"]);
        assert_eq!(s.description.to_plain(&lattice_proto::text::NoTranslations), "Hello, world!");
        assert!(!s.compatible());
    }
}
