//! Headless smoke test against a real server:
//!   cargo run -p lattice-net --example probe -- localhost:25566 Tester
//! Pings, joins in offline mode, waits for chunks, prints a summary.

use lattice_net::{ConnectOptions, Connection, Event, Profile, ServerAddress};
use lattice_proto::packets::play::{self, Cb};
use lattice_proto::text::NoTranslations;
use std::time::{Duration, Instant};

fn main() {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).init();
    let args: Vec<String> = std::env::args().collect();
    let addr = ServerAddress::parse(args.get(1).map(String::as_str).unwrap_or("localhost:25566")).unwrap();
    let name = args.get(2).cloned().unwrap_or_else(|| "Tester".into());
    let secs: u64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(8);

    match lattice_net::ping(&addr, Duration::from_secs(5)) {
        Ok(s) => println!(
            "status: {} (protocol {}), {}/{} players, {} ms, motd {:?}, favicon {} bytes",
            s.version_name,
            s.protocol,
            s.players_online.unwrap_or(-1),
            s.players_max.unwrap_or(-1),
            s.ping_ms,
            s.description.to_plain(&NoTranslations),
            s.favicon.as_ref().map_or(0, Vec::len)
        ),
        Err(e) => {
            println!("ping failed: {e}");
            return;
        }
    }

    let conn = Connection::connect(ConnectOptions {
        address: addr,
        profile: Profile::Offline { name },
        client_info: Default::default(),
        transfer: false,
    });
    let start = Instant::now();
    let (mut chunks, mut non_air_sections, mut play_packets) = (0, 0, 0);
    let mut kinds = std::collections::BTreeMap::<String, usize>::new();
    let mut ticks = 0;
    let mut pos = None;
    while start.elapsed() < Duration::from_secs(secs) {
        while let Some(ev) = conn.try_event() {
            match ev {
                Event::Stage(s) => println!("stage: {s:?}"),
                Event::LoggedIn(p) => println!("logged in as {} ({})", p.name, p.uuid),
                Event::Configured(r) => println!(
                    "configured: {} dimension types, {} biomes, {} chat types, {} tag registries",
                    r.dimension_types.len(),
                    r.biomes.len(),
                    r.chat_types.len(),
                    r.tags.len()
                ),
                Event::Chunk(c, _) => {
                    chunks += 1;
                    non_air_sections += c.sections.iter().filter(|s| !s.blocks.is_uniform_air()).count();
                }
                Event::Play(p) => {
                    play_packets += 1;
                    let k = format!("{:?}", p).split([' ', '(', '{']).next().unwrap().to_string();
                    *kinds.entry(k).or_default() += 1;
                    match *p {
                        Cb::Login(j) => println!("join: entity {}, dimension {}, game mode {}, online {}, secure chat {}", j.entity_id, j.spawn.dimension_name, j.spawn.game_mode, j.online_mode, j.enforces_secure_chat),
                        Cb::PlayerPosition { teleport_id, pos: p, yaw, pitch, .. } => {
                            println!("teleport {teleport_id} to {p:?}");
                            conn.send(play::accept_teleportation(teleport_id, p, yaw, pitch));
                            pos = Some((p, yaw, pitch));
                        }
                        Cb::SystemChat { content, .. } => println!("chat: {}", content.to_plain(&NoTranslations)),
                        _ => {}
                    }
                }
                Event::Disconnected(d) => {
                    println!("disconnected: {d:?}");
                    return;
                }
                Event::CodeOfConduct(c) => println!("code of conduct: {c}"),
                other => println!("event: {}", match other { Event::Notice(n) => n, Event::Reconfiguring => "reconfiguring".into(), _ => "other".into() }),
            }
        }
        if let Some((p, yaw, pitch)) = pos {
            // Stand still like a real client: status packets + tick end each tick.
            if ticks % 20 == 0 {
                conn.send(play::move_player_pos_rot(p, yaw, pitch, 0));
            }
            conn.send(play::client_tick_end());
            ticks += 1;
            if ticks == 20 {
                conn.send(play::player_loaded());
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    println!("chunks: {chunks}, non-air sections: {non_air_sections}, play packets: {play_packets}");
    println!("packet kinds: {kinds:?}");
    conn.close();
}
