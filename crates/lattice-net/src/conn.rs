//! A connection to a game server.
//!
//! The net thread resolves the address, logs in (handshake → login →
//! encryption/compression), then becomes the reader: it decodes packets,
//! answers keep-alives/pings/known-packs itself, decodes chunk data, and hands
//! everything else to the game as [`Event`]s. A separate writer thread owns the
//! socket's write half. Outgoing packets are tagged with the protocol state they
//! belong to; the writer drops packets that arrive for the wrong state (for
//! example a movement packet queued just as the server started reconfiguring).

use crate::address::{ServerAddress, resolve};
use crossbeam_channel::{Receiver, Sender, unbounded};
use lattice_proto::crypto;
use lattice_proto::frame::{FrameReader, FrameWriter};
use lattice_proto::packets::config::{self, ClientInformation};
use lattice_proto::packets::play::{self, BlockEntity};
use lattice_proto::packets::{Encoded, GameProfile, handshake, login};
use lattice_proto::{Reader, Text};
use lattice_world::chunk::{Column, Geometry};
use lattice_world::Registries;
use std::collections::VecDeque;
use std::io::{BufReader, ErrorKind};
use std::net::{Shutdown, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Called with the server hash; must POST to the session server's `join` endpoint.
pub type SessionJoin = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

#[derive(Clone)]
pub enum Profile {
    /// Offline-mode servers only.
    Offline { name: String },
    Online { name: String, uuid: Uuid, join: SessionJoin },
}

impl Profile {
    pub fn name(&self) -> &str {
        match self {
            Profile::Offline { name } | Profile::Online { name, .. } => name,
        }
    }
    pub fn uuid(&self) -> Uuid {
        match self {
            Profile::Offline { name } => lattice_proto::offline_uuid(name),
            Profile::Online { uuid, .. } => *uuid,
        }
    }
}

pub struct ConnectOptions {
    pub address: ServerAddress,
    pub profile: Profile,
    pub client_info: ClientInformation,
    /// Set when following a `transfer` packet.
    pub transfer: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Resolving,
    Connecting,
    Authenticating,
    LoggingIn,
    Configuring,
}

#[derive(Debug, Clone)]
pub enum Disconnect {
    /// The server closed the connection with a reason.
    Kicked(Text),
    /// We couldn't connect or log in; the string is user-facing.
    Failed(String),
    /// The connection dropped.
    Lost(String),
    /// We closed it.
    Closed,
}

pub enum Event {
    Stage(Stage),
    LoggedIn(GameProfile),
    CodeOfConduct(String),
    /// A server's resource pack, to fetch or decline with [`Connection::pack_status`].
    ResourcePackOffer { id: Uuid, url: String, hash: String, forced: bool },
    /// Stop using one pack, or all of them.
    ResourcePackRemoved(Option<Uuid>),
    ServerLinks(Vec<config::ServerLink>),
    Notice(String),
    /// Configuration finished; play packets follow.
    Configured(Arc<Registries>),
    Play(Box<play::Cb>),
    Chunk(Column, Vec<BlockEntity>),
    Reconfiguring,
    Transfer { host: String, port: i32 },
    Disconnected(Disconnect),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Configuration,
    Play,
}

enum Out {
    Packet(State, Encoded),
    /// Goes in whichever state the connection's in by then.
    PackStatus(Uuid, i32),
    SetState(State),
}

pub struct Connection {
    out: Sender<Out>,
    events: Receiver<Event>,
    stream: Arc<Mutex<Option<TcpStream>>>,
    closing: Arc<AtomicBool>,
}

const BRAND: &str = "lattice";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const READ_TIMEOUT: Duration = Duration::from_secs(30);

impl Connection {
    pub fn connect(opts: ConnectOptions) -> Connection {
        let (out_tx, out_rx) = unbounded();
        let (ev_tx, ev_rx) = unbounded();
        let stream = Arc::new(Mutex::new(None));
        let closing = Arc::new(AtomicBool::new(false));
        let ctx = NetThread { ev: ev_tx, out: out_tx.clone(), stream: stream.clone(), closing: closing.clone() };
        std::thread::Builder::new()
            .name("net".into())
            .spawn(move || {
                let result = ctx.run(opts, out_rx);
                let ev = match result {
                    Ok(()) => Disconnect::Closed,
                    Err(_) if ctx.closing.load(Ordering::SeqCst) => Disconnect::Closed,
                    Err(d) => d,
                };
                let _ = ctx.ev.send(Event::Disconnected(ev));
            })
            .expect("spawn net thread");
        Connection { out: out_tx, events: ev_rx, stream, closing }
    }

    /// Sends a play-state packet (dropped if the connection is reconfiguring).
    pub fn send(&self, p: Encoded) {
        let _ = self.out.send(Out::Packet(State::Play, p));
    }

    /// Sends a configuration-state packet.
    pub fn send_config(&self, p: Encoded) {
        let _ = self.out.send(Out::Packet(State::Configuration, p));
    }

    /// Tells the server how its resource pack is going.
    pub fn pack_status(&self, id: Uuid, status: i32) {
        let _ = self.out.send(Out::PackStatus(id, status));
    }

    pub fn try_event(&self) -> Option<Event> {
        self.events.try_recv().ok()
    }

    pub fn close(&self) {
        self.closing.store(true, Ordering::SeqCst);
        if let Some(s) = self.stream.lock().unwrap().as_ref() {
            let _ = s.shutdown(Shutdown::Both);
        }
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.close();
    }
}

struct NetThread {
    ev: Sender<Event>,
    out: Sender<Out>,
    stream: Arc<Mutex<Option<TcpStream>>>,
    closing: Arc<AtomicBool>,
}

type R = FrameReader<BufReader<TcpStream>>;
type W = FrameWriter<TcpStream>;

fn lost(e: std::io::Error) -> Disconnect {
    match e.kind() {
        ErrorKind::TimedOut | ErrorKind::WouldBlock => Disconnect::Lost("The server stopped responding (timed out).".into()),
        ErrorKind::UnexpectedEof | ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted => {
            Disconnect::Lost("The server closed the connection.".into())
        }
        _ => Disconnect::Lost(format!("Connection lost: {e}")),
    }
}

fn protocol_error(packet: &str, e: impl std::fmt::Display) -> Disconnect {
    Disconnect::Lost(format!("Lattice couldn't read the server's \"{packet}\" packet ({e}). This is a bug in Lattice."))
}

impl NetThread {
    fn emit(&self, e: Event) {
        let _ = self.ev.send(e);
    }

    fn send(&self, state: State, p: Encoded) {
        let _ = self.out.send(Out::Packet(state, p));
    }

    fn run(&self, opts: ConnectOptions, out_rx: Receiver<Out>) -> Result<(), Disconnect> {
        self.emit(Event::Stage(Stage::Resolving));
        let resolved = resolve(&opts.address).map_err(Disconnect::Failed)?;

        self.emit(Event::Stage(Stage::Connecting));
        let stream = resolved
            .addrs
            .iter()
            .find_map(|a| TcpStream::connect_timeout(a, CONNECT_TIMEOUT).ok())
            .ok_or_else(|| {
                Disconnect::Failed(format!(
                    "Couldn't reach {}. The server is offline or the address is wrong.",
                    opts.address.display()
                ))
            })?;
        stream.set_nodelay(true).ok();
        stream.set_read_timeout(Some(READ_TIMEOUT)).ok();
        *self.stream.lock().unwrap() = stream.try_clone().ok();
        if self.closing.load(Ordering::SeqCst) {
            return Ok(());
        }

        let mut w: W = FrameWriter::new(stream.try_clone().map_err(lost)?);
        let mut r: R = FrameReader::new(BufReader::with_capacity(1 << 16, stream));

        let intent = if opts.transfer { handshake::Intent::Transfer } else { handshake::Intent::Login };
        let p = handshake::intention(&resolved.handshake_host, resolved.handshake_port, intent);
        w.write_packet(p.id, &p.body).map_err(lost)?;
        let p = login::hello(opts.profile.name(), opts.profile.uuid());
        w.write_packet(p.id, &p.body).map_err(lost)?;
        self.emit(Event::Stage(Stage::LoggingIn));

        let profile = self.login(&opts, &mut r, &mut w)?;
        self.emit(Event::LoggedIn(profile));
        self.emit(Event::Stage(Stage::Configuring));

        // Our first configuration packets go out before the writer thread starts.
        for p in [
            config::client_information(&opts.client_info),
            config::custom_payload("minecraft:brand", &brand_payload()),
        ] {
            w.write_packet(p.id, &p.body).map_err(lost)?;
        }

        std::thread::Builder::new()
            .name("net-writer".into())
            .spawn(move || writer(w, out_rx))
            .map_err(|e| Disconnect::Failed(e.to_string()))?;

        self.read_loop(&mut r)
    }

    fn login(&self, opts: &ConnectOptions, r: &mut R, w: &mut W) -> Result<GameProfile, Disconnect> {
        loop {
            let frame = r.read_frame().map_err(lost)?;
            let mut rd = Reader::new(&frame);
            let id = rd.var_i32().map_err(|e| protocol_error("login", e))?;
            let packet = login::decode(id, &mut rd).map_err(|e| protocol_error(login_name(id), e))?;
            match packet {
                login::Cb::Disconnect(reason) => return Err(Disconnect::Kicked(reason)),
                login::Cb::EncryptionRequest { server_id, public_key, verify_token, should_authenticate } => {
                    let mut secret = [0u8; 16];
                    rand::fill(&mut secret);
                    if should_authenticate {
                        match &opts.profile {
                            Profile::Online { join, .. } => {
                                self.emit(Event::Stage(Stage::Authenticating));
                                join(&crypto::server_hash(&server_id, &secret, &public_key)).map_err(Disconnect::Failed)?;
                            }
                            Profile::Offline { .. } => {
                                return Err(Disconnect::Failed(
                                    "This server checks accounts with Mojang. Sign in with your Microsoft account to join it.".into(),
                                ));
                            }
                        }
                    }
                    let bad_key = |_| Disconnect::Failed("The server's encryption key is invalid.".into());
                    let enc_secret = crypto::rsa_encrypt(&public_key, &secret).map_err(bad_key)?;
                    let enc_token = crypto::rsa_encrypt(&public_key, &verify_token).map_err(bad_key)?;
                    let p = login::encryption_response(&enc_secret, &enc_token);
                    w.write_packet(p.id, &p.body).map_err(lost)?;
                    let (e, d) = crypto::ciphers(&secret);
                    w.enable_encryption(e);
                    r.enable_encryption(d);
                }
                login::Cb::SetCompression(t) => {
                    r.set_compression(t);
                    w.set_compression(t);
                }
                login::Cb::LoginSuccess { profile, .. } => {
                    let p = login::login_acknowledged();
                    w.write_packet(p.id, &p.body).map_err(lost)?;
                    return Ok(profile);
                }
                login::Cb::PluginRequest { message_id, .. } => {
                    let p = login::plugin_response_not_understood(message_id);
                    w.write_packet(p.id, &p.body).map_err(lost)?;
                }
                login::Cb::CookieRequest(key) => {
                    let p = login::cookie_response_empty(&key);
                    w.write_packet(p.id, &p.body).map_err(lost)?;
                }
                login::Cb::Unknown(id) => tracing::warn!("unknown login packet {id:#x}"),
            }
        }
    }

    fn read_loop(&self, r: &mut R) -> Result<(), Disconnect> {
        let data = lattice_data::game();
        let mut state = State::Configuration;
        let mut registries = Registries::default();
        let mut shared: Option<Arc<Registries>> = None;
        let mut geometry: Option<Geometry> = None;
        let mut batch_start: Option<Instant> = None;
        let mut ms_per_chunk: VecDeque<f64> = VecDeque::new();

        loop {
            let frame = r.read_frame().map_err(lost)?;
            let mut rd = Reader::new(&frame);
            let id = rd.var_i32().map_err(|e| protocol_error("?", e))?;
            match state {
                State::Configuration => {
                    let p = config::decode(id, &mut rd)
                        .map_err(|e| protocol_error(lattice_proto::ids::configuration::cb::name(id), e))?;
                    match p {
                        config::Cb::KeepAlive(k) => self.send(State::Configuration, config::keep_alive(k)),
                        config::Cb::Ping(k) => self.send(State::Configuration, config::pong(k)),
                        config::Cb::RegistryData { registry, entries } => registries.insert(&registry, entries),
                        config::Cb::UpdateTags(tags) => registries.set_tags(tags),
                        config::Cb::SelectKnownPacks(_) => {
                            // We know no packs, so the server sends full registry data.
                            self.send(State::Configuration, config::select_known_packs(&[]));
                        }
                        config::Cb::FinishConfiguration => {
                            self.send(State::Configuration, config::finish_configuration());
                            let _ = self.out.send(Out::SetState(State::Play));
                            state = State::Play;
                            let reg = Arc::new(std::mem::take(&mut registries));
                            shared = Some(reg.clone());
                            self.emit(Event::Configured(reg));
                        }
                        config::Cb::CodeOfConduct(text) => self.emit(Event::CodeOfConduct(text)),
                        config::Cb::ServerLinks(links) => self.emit(Event::ServerLinks(links)),
                        config::Cb::ResourcePackPush(pack) => self.emit(Event::ResourcePackOffer { id: pack.uuid, url: pack.url, hash: pack.hash, forced: pack.forced }),
                        config::Cb::ResourcePackPop(id) => self.emit(Event::ResourcePackRemoved(id)),
                        config::Cb::CookieRequest(key) => self.send(State::Configuration, config::cookie_response_empty(&key)),
                        config::Cb::Transfer { host, port } => {
                            self.emit(Event::Transfer { host, port });
                            return Ok(());
                        }
                        config::Cb::Disconnect(reason) => return Err(Disconnect::Kicked(reason)),
                        config::Cb::ShowDialog(_) => {
                            self.emit(Event::Notice("The server opened a dialog window. Lattice can't show those yet.".into()))
                        }
                        config::Cb::CustomPayload { channel, .. } => tracing::debug!("plugin message on {channel}"),
                        config::Cb::Unknown(id) => tracing::debug!("unhandled configuration packet {id:#x}"),
                        _ => {}
                    }
                }
                State::Play => {
                    let name = lattice_proto::ids::play::cb::name(id);
                    let p = play::decode(id, &mut rd).map_err(|e| {
                        dump_frame(name, &frame);
                        protocol_error(name, e)
                    })?;
                    if rd.remaining() > 0 && !matches!(p, play::Cb::Unknown(_)) {
                        tracing::warn!("{} bytes left over after decoding {name}", rd.remaining());
                    }
                    match p {
                        play::Cb::KeepAlive(k) => self.send(State::Play, play::keep_alive(k)),
                        play::Cb::Ping(k) => self.send(State::Play, play::pong(k)),
                        play::Cb::ChunkBatchStart => batch_start = Some(Instant::now()),
                        play::Cb::ChunkBatchFinished(n) => {
                            if let (Some(t), true) = (batch_start.take(), n > 0) {
                                ms_per_chunk.push_back(t.elapsed().as_secs_f64() * 1000.0 / n as f64);
                                if ms_per_chunk.len() > 15 {
                                    ms_per_chunk.pop_front();
                                }
                            }
                            let avg = if ms_per_chunk.is_empty() {
                                1.0
                            } else {
                                ms_per_chunk.iter().sum::<f64>() / ms_per_chunk.len() as f64
                            };
                            let rate = (25.0 / avg.max(1e-3)).clamp(0.01, 64.0);
                            self.send(State::Play, play::chunk_batch_received(rate as f32));
                        }
                        play::Cb::LevelChunk(chunk) => {
                            let Some(geo) = geometry else {
                                tracing::warn!("chunk before login; skipped");
                                continue;
                            };
                            let biome_bits = shared.as_ref().map_or(7, |r| r.biome_bits());
                            match Column::decode(&chunk, geo, data.block_state_bits(), biome_bits) {
                                Ok(col) => self.emit(Event::Chunk(col, chunk.block_entities)),
                                Err(e) => tracing::warn!("chunk {},{} failed to decode: {e}", chunk.x, chunk.z),
                            }
                        }
                        play::Cb::Login(ref j) => {
                            geometry = dimension_geometry(shared.as_deref(), j.spawn.dimension_type);
                            self.emit(Event::Play(Box::new(p)));
                        }
                        play::Cb::Respawn { ref spawn, .. } => {
                            geometry = dimension_geometry(shared.as_deref(), spawn.dimension_type);
                            self.emit(Event::Play(Box::new(p)));
                        }
                        play::Cb::StartConfiguration => {
                            self.send(State::Play, play::configuration_acknowledged());
                            let _ = self.out.send(Out::SetState(State::Configuration));
                            state = State::Configuration;
                            registries = Registries::default();
                            self.emit(Event::Reconfiguring);
                        }
                        play::Cb::ResourcePackPush(pack) => self.emit(Event::ResourcePackOffer { id: pack.uuid, url: pack.url, hash: pack.hash, forced: pack.forced }),
                        play::Cb::ResourcePackPop(id) => self.emit(Event::ResourcePackRemoved(id)),
                        play::Cb::Disconnect(reason) => return Err(Disconnect::Kicked(reason)),
                        play::Cb::Unknown(_) => {}
                        other => self.emit(Event::Play(Box::new(other))),
                    }
                }
            }
        }
    }
}

/// With `LATTICE_DUMP_DIR` set, undecodable packets are written there for debugging.
fn dump_frame(name: &str, frame: &[u8]) {
    if let Some(dir) = std::env::var_os("LATTICE_DUMP_DIR") {
        let path = std::path::Path::new(&dir).join(format!("{}.bin", name.replace('/', "_")));
        if std::fs::write(&path, frame).is_ok() {
            tracing::warn!("wrote undecodable {name} packet to {}", path.display());
        }
    }
}

fn dimension_geometry(reg: Option<&Registries>, dimension_type: i32) -> Option<Geometry> {
    let d = reg?.dimension_types.get(dimension_type as usize)?;
    Some(Geometry { min_y: d.min_y, height: d.height })
}

fn login_name(id: i32) -> &'static str {
    lattice_proto::ids::login::cb::name(id)
}

fn brand_payload() -> Vec<u8> {
    let mut w = lattice_proto::Writer::new();
    w.string(BRAND);
    w.buf
}

fn writer(mut w: W, rx: Receiver<Out>) {
    let mut state = State::Configuration;
    for msg in rx {
        match msg {
            Out::SetState(s) => state = s,
            Out::PackStatus(id, status) => {
                let p = match state {
                    State::Configuration => config::resource_pack_response(id, status),
                    State::Play => play::resource_pack_response(id, status),
                };
                if let Err(e) = w.write_packet(p.id, &p.body) {
                    tracing::debug!("writer stopped: {e}");
                    return;
                }
            }
            Out::Packet(s, p) if s == state => {
                if let Err(e) = w.write_packet(p.id, &p.body) {
                    tracing::debug!("writer stopped: {e}");
                    return;
                }
            }
            Out::Packet(s, p) => tracing::debug!("dropped packet {:#x} for {s:?} while in {state:?}", p.id),
        }
    }
}
