//! Server addresses as typed by users: "host", "host:port", "[v6]:port".
//! Without an explicit port, the `_minecraft._tcp` SRV record is consulted,
//! as the vanilla client does.

use std::net::{SocketAddr, ToSocketAddrs};

pub const DEFAULT_PORT: u16 = 25565;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerAddress {
    pub host: String,
    pub port: u16,
    pub explicit_port: bool,
}

impl ServerAddress {
    pub fn parse(input: &str) -> Result<Self, String> {
        let s = input.trim();
        if s.is_empty() {
            return Err("Enter a server address.".into());
        }
        let (host, port) = if let Some(rest) = s.strip_prefix('[') {
            let end = rest.find(']').ok_or("The IPv6 address is missing its closing bracket.")?;
            let host = &rest[..end];
            let after = &rest[end + 1..];
            (host.to_string(), after.strip_prefix(':').map(str::to_string))
        } else if s.matches(':').count() == 1 {
            let (h, p) = s.split_once(':').unwrap();
            (h.to_string(), Some(p.to_string()))
        } else {
            (s.to_string(), None)
        };
        if host.is_empty() {
            return Err("The address has no host name.".into());
        }
        match port {
            Some(p) => {
                let port: u16 = p.parse().map_err(|_| format!("\"{p}\" isn't a valid port (1–65535)."))?;
                if port == 0 {
                    return Err("Port 0 isn't valid.".into());
                }
                Ok(Self { host, port, explicit_port: true })
            }
            None => Ok(Self { host, port: DEFAULT_PORT, explicit_port: false }),
        }
    }

    pub fn display(&self) -> String {
        if self.explicit_port && self.port != DEFAULT_PORT {
            if self.host.contains(':') { format!("[{}]:{}", self.host, self.port) } else { format!("{}:{}", self.host, self.port) }
        } else {
            self.host.clone()
        }
    }
}

/// Where to actually connect, and the host/port to put in the handshake.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub addrs: Vec<SocketAddr>,
    pub handshake_host: String,
    pub handshake_port: u16,
}

fn srv_lookup(host: &str) -> Option<(String, u16)> {
    if host.parse::<std::net::IpAddr>().is_ok() || host.eq_ignore_ascii_case("localhost") {
        return None;
    }
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().ok()?;
    rt.block_on(async {
        let resolver = hickory_resolver::TokioResolver::builder_tokio().ok()?.build().ok()?;
        let lookup = tokio::time::timeout(
            std::time::Duration::from_secs(4),
            resolver.srv_lookup(format!("_minecraft._tcp.{host}.")),
        )
        .await
        .ok()?
        .ok()?;
        let mut best: Option<(u16, u16, String, u16)> = None;
        for rec in lookup.answers() {
            if let hickory_resolver::proto::rr::RData::SRV(srv) = &rec.data {
                let target = srv.target.to_utf8().trim_end_matches('.').to_string();
                let cand = (srv.priority, u16::MAX - srv.weight, target, srv.port);
                if best.as_ref().is_none_or(|b| (cand.0, cand.1) < (b.0, b.1)) {
                    best = Some(cand);
                }
            }
        }
        best.map(|(_, _, t, p)| (t, p))
    })
}

pub fn resolve(addr: &ServerAddress) -> Result<Resolved, String> {
    let (host, port) = if addr.explicit_port {
        (addr.host.clone(), addr.port)
    } else {
        srv_lookup(&addr.host).unwrap_or((addr.host.clone(), addr.port))
    };
    let addrs: Vec<SocketAddr> = (host.as_str(), port)
        .to_socket_addrs()
        .map_err(|_| format!("Couldn't find a server called {}. Check the address for typos.", addr.host))?
        .collect();
    if addrs.is_empty() {
        return Err(format!("Couldn't find a server called {}.", addr.host));
    }
    Ok(Resolved { addrs, handshake_host: host, handshake_port: port })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsing() {
        assert_eq!(ServerAddress::parse("play.example.net").unwrap(), ServerAddress { host: "play.example.net".into(), port: 25565, explicit_port: false });
        assert_eq!(ServerAddress::parse(" localhost:25566 ").unwrap().port, 25566);
        assert_eq!(ServerAddress::parse("[::1]:1234").unwrap(), ServerAddress { host: "::1".into(), port: 1234, explicit_port: true });
        assert_eq!(ServerAddress::parse("::1").unwrap().host, "::1");
        assert!(ServerAddress::parse("host:99999").is_err());
        assert!(ServerAddress::parse("").is_err());
        assert_eq!(ServerAddress::parse("a.b:25565").unwrap().display(), "a.b");
        assert_eq!(ServerAddress::parse("a.b:1").unwrap().display(), "a.b:1");
    }
}
