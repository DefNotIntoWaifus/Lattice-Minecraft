use super::{Encoded, enc};
use crate::codec::{DResult, Reader, Writer};
use crate::ids::configuration::{cb, sb};
use crate::nbt::Nbt;
use crate::text::Text;
use uuid::Uuid;

/// A link a server offers, such as its website or rules.
#[derive(Debug, Clone)]
pub struct ServerLink {
    /// The game's own kind of link (0 report a bug, 1 guidelines, 2 support, 3 status,
    /// 4 feedback, 5 community, 6 website, 7 forums, 8 news, 9 announcements).
    pub known: Option<i32>,
    pub label: Option<Text>,
    pub url: String,
}

pub(crate) fn read_server_links(r: &mut Reader) -> DResult<Vec<ServerLink>> {
    r.array(|r| {
        let (known, label) = if r.bool()? { (Some(r.var_i32()?), None) } else { (None, Some(r.text()?)) };
        Ok(ServerLink { known, label, url: r.string(32767)? })
    })
}

#[derive(Debug)]
pub enum Cb {
    ServerLinks(Vec<ServerLink>),
    CookieRequest(String),
    CustomPayload { channel: String, data: Vec<u8> },
    Disconnect(Text),
    FinishConfiguration,
    KeepAlive(i64),
    Ping(i32),
    ResetChat,
    RegistryData { registry: String, entries: Vec<(String, Option<Nbt>)> },
    ResourcePackPop(Option<Uuid>),
    ResourcePackPush(ResourcePack),
    StoreCookie,
    Transfer { host: String, port: i32 },
    UpdateEnabledFeatures(Vec<String>),
    UpdateTags(Tags),
    SelectKnownPacks(Vec<KnownPack>),
    ClearDialog,
    ShowDialog(Nbt),
    CodeOfConduct(String),
    /// Report details, server links, post effects: accepted and ignored.
    Ignored(i32),
    Unknown(i32),
}

#[derive(Debug, Clone)]
pub struct ResourcePack {
    pub uuid: Uuid,
    pub url: String,
    pub hash: String,
    pub forced: bool,
    pub prompt: Option<Text>,
}

#[derive(Debug, Clone)]
pub struct KnownPack {
    pub namespace: String,
    pub id: String,
    pub version: String,
}

/// registry → (tag name → entry IDs)
pub type Tags = Vec<(String, Vec<(String, Vec<i32>)>)>;

pub(crate) fn read_tags(r: &mut Reader) -> DResult<Tags> {
    r.array(|r| Ok((r.identifier()?, r.array(|r| Ok((r.identifier()?, r.array(|r| r.var_i32())?)))?)))
}

pub(crate) fn read_resource_pack(r: &mut Reader) -> DResult<ResourcePack> {
    Ok(ResourcePack {
        uuid: r.uuid()?,
        url: r.string(32767)?,
        hash: r.string(40)?,
        forced: r.bool()?,
        prompt: r.option(|r| r.text())?,
    })
}

pub fn decode(id: i32, r: &mut Reader) -> DResult<Cb> {
    Ok(match id {
        cb::COOKIE_REQUEST => Cb::CookieRequest(r.identifier()?),
        cb::CUSTOM_PAYLOAD => Cb::CustomPayload { channel: r.identifier()?, data: r.rest().to_vec() },
        cb::DISCONNECT => Cb::Disconnect(r.text()?),
        cb::FINISH_CONFIGURATION => Cb::FinishConfiguration,
        cb::KEEP_ALIVE => Cb::KeepAlive(r.i64()?),
        cb::PING => Cb::Ping(r.i32()?),
        cb::RESET_CHAT => Cb::ResetChat,
        cb::REGISTRY_DATA => Cb::RegistryData {
            registry: r.identifier()?,
            entries: r.array(|r| Ok((r.identifier()?, r.option(|r| r.nbt())?)))?,
        },
        cb::RESOURCE_PACK_POP => Cb::ResourcePackPop(r.option(|r| r.uuid())?),
        cb::RESOURCE_PACK_PUSH => Cb::ResourcePackPush(read_resource_pack(r)?),
        cb::STORE_COOKIE => {
            r.rest();
            Cb::StoreCookie
        }
        cb::TRANSFER => Cb::Transfer { host: r.string(32767)?, port: r.var_i32()? },
        cb::UPDATE_ENABLED_FEATURES => Cb::UpdateEnabledFeatures(r.array(|r| r.identifier())?),
        cb::UPDATE_TAGS => Cb::UpdateTags(read_tags(r)?),
        cb::SELECT_KNOWN_PACKS => Cb::SelectKnownPacks(r.array(|r| {
            Ok(KnownPack { namespace: r.string(32767)?, id: r.string(32767)?, version: r.string(32767)? })
        })?),
        cb::CLEAR_DIALOG => Cb::ClearDialog,
        cb::SHOW_DIALOG => Cb::ShowDialog(r.nbt()?),
        cb::CODE_OF_CONDUCT => Cb::CodeOfConduct(r.string(32767)?),
        cb::SERVER_LINKS => Cb::ServerLinks(read_server_links(r)?),
        cb::CUSTOM_REPORT_DETAILS | cb::POST_EFFECTS => {
            r.rest();
            Cb::Ignored(id)
        }
        other => Cb::Unknown(other),
    })
}

/// `Client Information`, shared by configuration and play.
#[derive(Debug, Clone)]
pub struct ClientInformation {
    pub locale: String,
    pub view_distance: u8,
    /// 0 enabled, 1 commands only, 2 hidden
    pub chat_mode: i32,
    pub chat_colors: bool,
    pub skin_parts: u8,
    /// 0 left, 1 right
    pub main_hand: i32,
    pub text_filtering: bool,
    pub allow_listing: bool,
    /// 0 all, 1 decreased, 2 minimal
    pub particles: i32,
}

impl Default for ClientInformation {
    fn default() -> Self {
        Self {
            locale: "en_us".into(),
            view_distance: 10,
            chat_mode: 0,
            chat_colors: true,
            skin_parts: 0x7F,
            main_hand: 1,
            text_filtering: false,
            allow_listing: true,
            particles: 0,
        }
    }
}

impl ClientInformation {
    pub(crate) fn write(&self, w: &mut Writer) {
        w.string(&self.locale)
            .u8(self.view_distance)
            .var_i32(self.chat_mode)
            .bool(self.chat_colors)
            .u8(self.skin_parts)
            .var_i32(self.main_hand)
            .bool(self.text_filtering)
            .bool(self.allow_listing)
            .var_i32(self.particles);
    }
}

pub fn client_information(info: &ClientInformation) -> Encoded {
    enc(sb::CLIENT_INFORMATION, |w| info.write(w))
}

pub fn cookie_response_empty(key: &str) -> Encoded {
    enc(sb::COOKIE_RESPONSE, |w| {
        w.string(key).bool(false);
    })
}

/// `minecraft:brand` and other plugin channels.
pub fn custom_payload(channel: &str, data: &[u8]) -> Encoded {
    enc(sb::CUSTOM_PAYLOAD, |w| {
        w.string(channel).bytes(data);
    })
}

pub fn finish_configuration() -> Encoded {
    enc(sb::FINISH_CONFIGURATION, |_| {})
}

pub fn keep_alive(id: i64) -> Encoded {
    enc(sb::KEEP_ALIVE, |w| {
        w.i64(id);
    })
}

pub fn pong(id: i32) -> Encoded {
    enc(sb::PONG, |w| {
        w.i32(id);
    })
}

/// Resource pack result: 0 loaded, 1 declined, 2 failed download, 3 accepted,
/// 4 downloaded, 5 invalid URL, 6 failed reload, 7 discarded.
pub fn resource_pack_response(uuid: Uuid, result: i32) -> Encoded {
    enc(sb::RESOURCE_PACK, |w| {
        w.uuid(uuid).var_i32(result);
    })
}

pub fn select_known_packs(packs: &[KnownPack]) -> Encoded {
    enc(sb::SELECT_KNOWN_PACKS, |w| {
        w.var_i32(packs.len() as i32);
        for p in packs {
            w.string(&p.namespace).string(&p.id).string(&p.version);
        }
    })
}

pub fn accept_code_of_conduct() -> Encoded {
    enc(sb::ACCEPT_CODE_OF_CONDUCT, |_| {})
}
