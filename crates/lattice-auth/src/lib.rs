//! Microsoft account sign-in for Minecraft: Java Edition.
//!
//! Device code flow → Microsoft (MSA) token → Xbox Live user token → XSTS
//! token for `rp://api.minecraftservices.com/` → Minecraft access token →
//! entitlement and profile checks. Also the session-server `join` call used
//! when connecting to online-mode servers.
//!
//! The Azure application (client) ID must be approved by Mojang to call
//! `login_with_xbox`; unapproved IDs get HTTP 403, reported as
//! [`AuthError::AppNotApproved`]. See docs/DESIGN.md §7.2.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::{Duration, SystemTime};
use uuid::Uuid;

const DEVICE_CODE_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode";
const TOKEN_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
const XBL_URL: &str = "https://user.auth.xboxlive.com/user/authenticate";
const XSTS_URL: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";
const MC_LOGIN_URL: &str = "https://api.minecraftservices.com/authentication/login_with_xbox";
const ENTITLEMENTS_URL: &str = "https://api.minecraftservices.com/entitlements/mcstore";
const PROFILE_URL: &str = "https://api.minecraftservices.com/minecraft/profile";
const JOIN_URL: &str = "https://sessionserver.mojang.com/session/minecraft/join";
const SCOPE: &str = "XboxLive.signin offline_access";

#[derive(Debug, Clone, thiserror::Error, PartialEq)]
pub enum AuthError {
    #[error("No Microsoft application ID is configured. Set [auth] client_id in config.toml or LATTICE_CLIENT_ID.")]
    NoClientId,
    #[error("Couldn't reach {0}. Check your internet connection.")]
    Network(String),
    #[error("Sign-in was declined on the Microsoft page.")]
    Declined,
    #[error("The sign-in code expired before it was used. Start again to get a new code.")]
    Expired,
    #[error("Microsoft rejected this application ID: {0}")]
    BadClient(String),
    #[error("Your saved sign-in is no longer valid. Sign in again.")]
    RefreshRejected,
    #[error("This Microsoft account has no Xbox profile yet. Sign in once at minecraft.net to create it, then try again.")]
    NoXboxAccount,
    #[error("This account belongs to someone under 18. An adult has to add it to a Microsoft family group first.")]
    Child,
    #[error("Xbox Live isn't available in this account's country or region.")]
    Region,
    #[error("This account needs adult verification on the Xbox website (required in South Korea).")]
    AdultVerification,
    #[error("This account is banned from Xbox Live.")]
    Banned,
    #[error("Xbox Live refused the sign-in (error {0}).")]
    Xbox(u64),
    #[error("Mojang hasn't approved this application ID for Minecraft sign-in yet, so Microsoft accounts can't be used. Offline-mode servers still work.")]
    AppNotApproved,
    #[error("Too many sign-in attempts. Wait a few minutes and try again.")]
    RateLimited,
    #[error("This Microsoft account doesn't own Minecraft: Java Edition.")]
    NoGame,
    #[error("This account owns the game but has no Java Edition profile. Open the official launcher once to choose a name.")]
    NoProfile,
    #[error("Unexpected answer from {service}: {detail}")]
    Unexpected { service: &'static str, detail: String },
}

type Result<T> = std::result::Result<T, AuthError>;

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(20)))
        .user_agent(concat!("lattice/", env!("CARGO_PKG_VERSION")))
        .build()
        .new_agent()
}

fn host(url: &str) -> String {
    url.split('/').nth(2).unwrap_or(url).to_string()
}

fn read(resp: Result<ureq::http::Response<ureq::Body>>, service: &'static str) -> Result<(u16, Value)> {
    let mut resp = resp?;
    let status = resp.status().as_u16();
    let text = resp.body_mut().with_config().limit(1 << 20).read_to_string().unwrap_or_default();
    let body = if text.trim().is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&text).map_err(|_| AuthError::Unexpected { service, detail: format!("HTTP {status}, not JSON") })?
    };
    Ok((status, body))
}

fn post_form(url: &str, form: &[(&str, &str)]) -> Result<ureq::http::Response<ureq::Body>> {
    agent().post(url).send_form(form.iter().copied()).map_err(|_| AuthError::Network(host(url)))
}

fn post_json(url: &str, body: &Value) -> Result<ureq::http::Response<ureq::Body>> {
    agent()
        .post(url)
        .header("Accept", "application/json")
        .send_json(body)
        .map_err(|_| AuthError::Network(host(url)))
}

fn get_bearer(url: &str, token: &str) -> Result<ureq::http::Response<ureq::Body>> {
    agent()
        .get(url)
        .header("Authorization", &format!("Bearer {token}"))
        .header("Accept", "application/json")
        .call()
        .map_err(|_| AuthError::Network(host(url)))
}

// ------------------------------------------------------------ Microsoft

#[derive(Debug, Clone, Deserialize)]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Debug, Clone)]
pub struct MsaTokens {
    pub access_token: String,
    pub refresh_token: String,
}

fn oauth_error(body: &Value) -> AuthError {
    let code = body["error"].as_str().unwrap_or("");
    match code {
        "authorization_declined" | "access_denied" => AuthError::Declined,
        "expired_token" | "bad_verification_code" => AuthError::Expired,
        "invalid_grant" => AuthError::RefreshRejected,
        "invalid_client" | "unauthorized_client" | "invalid_request" => {
            AuthError::BadClient(body["error_description"].as_str().unwrap_or(code).lines().next().unwrap_or("").to_string())
        }
        _ => AuthError::Unexpected { service: "Microsoft", detail: code.to_string() },
    }
}

pub fn start_device_code(client_id: &str) -> Result<DeviceCode> {
    if client_id.trim().is_empty() {
        return Err(AuthError::NoClientId);
    }
    let (status, body) = read(post_form(DEVICE_CODE_URL, &[("client_id", client_id), ("scope", SCOPE)]), "Microsoft")?;
    if status != 200 {
        return Err(oauth_error(&body));
    }
    serde_json::from_value(body).map_err(|e| AuthError::Unexpected { service: "Microsoft", detail: e.to_string() })
}

pub enum Poll {
    Pending,
    /// The server asked us to poll less often.
    SlowDown,
    Done(MsaTokens),
}

pub fn poll_device_code(client_id: &str, code: &DeviceCode) -> Result<Poll> {
    let (status, body) = read(
        post_form(
            TOKEN_URL,
            &[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("client_id", client_id),
                ("device_code", &code.device_code),
            ],
        ),
        "Microsoft",
    )?;
    if status == 200 {
        return tokens(&body).map(Poll::Done);
    }
    match body["error"].as_str() {
        Some("authorization_pending") => Ok(Poll::Pending),
        Some("slow_down") => Ok(Poll::SlowDown),
        _ => Err(oauth_error(&body)),
    }
}

fn tokens(body: &Value) -> Result<MsaTokens> {
    match (body["access_token"].as_str(), body["refresh_token"].as_str()) {
        (Some(a), Some(r)) => Ok(MsaTokens { access_token: a.to_string(), refresh_token: r.to_string() }),
        _ => Err(AuthError::Unexpected { service: "Microsoft", detail: "token response without tokens".into() }),
    }
}

pub fn refresh(client_id: &str, refresh_token: &str) -> Result<MsaTokens> {
    let (status, body) = read(
        post_form(
            TOKEN_URL,
            &[("grant_type", "refresh_token"), ("client_id", client_id), ("refresh_token", refresh_token), ("scope", SCOPE)],
        ),
        "Microsoft",
    )?;
    if status != 200 {
        return Err(oauth_error(&body));
    }
    tokens(&body)
}

// ------------------------------------------------------------ Xbox → Minecraft

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    XboxLive,
    Xsts,
    Minecraft,
    Ownership,
    Profile,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub id: Uuid,
    pub name: String,
    pub skin_url: Option<String>,
    /// "CLASSIC" or "SLIM"
    pub skin_variant: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Session {
    pub access_token: String,
    pub expires_at: SystemTime,
    pub profile: Profile,
}

impl Session {
    /// True when the token expires within five minutes.
    pub fn needs_refresh(&self) -> bool {
        self.expires_at.duration_since(SystemTime::now()).map_or(true, |d| d < Duration::from_secs(300))
    }
}

fn xerr(body: &Value) -> AuthError {
    match body["XErr"].as_u64().unwrap_or(0) {
        2148916227 => AuthError::Banned,
        2148916233 => AuthError::NoXboxAccount,
        2148916235 => AuthError::Region,
        2148916236 | 2148916237 => AuthError::AdultVerification,
        2148916238 => AuthError::Child,
        other => AuthError::Xbox(other),
    }
}

/// Exchanges a Microsoft access token for a Minecraft session.
pub fn minecraft_session(msa_access_token: &str, step: &dyn Fn(Step)) -> Result<Session> {
    step(Step::XboxLive);
    let (status, xbl) = read(
        post_json(
            XBL_URL,
            &json!({
                "Properties": {"AuthMethod": "RPS", "SiteName": "user.auth.xboxlive.com", "RpsTicket": format!("d={msa_access_token}")},
                "RelyingParty": "http://auth.xboxlive.com",
                "TokenType": "JWT"
            }),
        ),
        "Xbox Live",
    )?;
    if status != 200 {
        return Err(if status == 401 { xerr(&xbl) } else { AuthError::Unexpected { service: "Xbox Live", detail: format!("HTTP {status}") } });
    }
    let xbl_token = xbl["Token"].as_str().unwrap_or_default();

    step(Step::Xsts);
    let (status, xsts) = read(
        post_json(
            XSTS_URL,
            &json!({
                "Properties": {"SandboxId": "RETAIL", "UserTokens": [xbl_token]},
                "RelyingParty": "rp://api.minecraftservices.com/",
                "TokenType": "JWT"
            }),
        ),
        "Xbox Live",
    )?;
    if status != 200 {
        return Err(if status == 401 { xerr(&xsts) } else { AuthError::Unexpected { service: "Xbox Live", detail: format!("HTTP {status}") } });
    }
    let xsts_token = xsts["Token"].as_str().unwrap_or_default();
    let uhs = xsts["DisplayClaims"]["xui"][0]["uhs"].as_str().unwrap_or_default();

    step(Step::Minecraft);
    let (status, mc) = read(post_json(MC_LOGIN_URL, &json!({"identityToken": format!("XBL3.0 x={uhs};{xsts_token}")})), "Minecraft services")?;
    match status {
        200 => {}
        403 => return Err(AuthError::AppNotApproved),
        429 => return Err(AuthError::RateLimited),
        s => return Err(AuthError::Unexpected { service: "Minecraft services", detail: format!("HTTP {s}") }),
    }
    let access = mc["access_token"].as_str().unwrap_or_default().to_string();
    let expires_in = mc["expires_in"].as_u64().unwrap_or(86_400);

    step(Step::Ownership);
    let (status, ent) = read(get_bearer(ENTITLEMENTS_URL, &access), "Minecraft services")?;
    if status == 200 {
        let owns = ent["items"]
            .as_array()
            .is_some_and(|items| items.iter().any(|i| matches!(i["name"].as_str(), Some("game_minecraft") | Some("product_minecraft"))));
        // Game Pass entitlements aren't always listed here; the profile check below is decisive.
        if !owns {
            tracing::info!("no Java entitlement listed; checking the profile anyway (Game Pass)");
        }
    }

    step(Step::Profile);
    let (status, p) = read(get_bearer(PROFILE_URL, &access), "Minecraft services")?;
    match status {
        200 => {}
        404 => {
            let owns = ent["items"].as_array().is_some_and(|i| !i.is_empty());
            return Err(if owns { AuthError::NoProfile } else { AuthError::NoGame });
        }
        429 => return Err(AuthError::RateLimited),
        s => return Err(AuthError::Unexpected { service: "Minecraft services", detail: format!("HTTP {s}") }),
    }
    let id = p["id"].as_str().and_then(|s| Uuid::parse_str(s).ok()).ok_or(AuthError::Unexpected {
        service: "Minecraft services",
        detail: "profile without an id".into(),
    })?;
    let active_skin = p["skins"].as_array().and_then(|s| s.iter().find(|s| s["state"] == "ACTIVE"));
    Ok(Session {
        access_token: access,
        expires_at: SystemTime::now() + Duration::from_secs(expires_in),
        profile: Profile {
            id,
            name: p["name"].as_str().unwrap_or_default().to_string(),
            skin_url: active_skin.and_then(|s| s["url"].as_str()).map(str::to_owned),
            skin_variant: active_skin.and_then(|s| s["variant"].as_str()).map(str::to_owned),
        },
    })
}

/// The keys chat gets signed with, as Mojang hands them out.
#[derive(Debug, Clone)]
pub struct ProfileKeys {
    pub private_key_pem: String,
    pub public_key_pem: String,
    /// Mojang's word for the public key, base64.
    pub key_signature: String,
    pub expires_at: String,
}

/// Asks for this player's chat keys.
pub fn profile_keys(access_token: &str) -> std::result::Result<ProfileKeys, String> {
    let resp = agent()
        .post("https://api.minecraftservices.com/player/certificates")
        .header("Authorization", &format!("Bearer {access_token}"))
        .header("Accept", "application/json")
        .send_empty()
        .map_err(|_| AuthError::Network(host("https://api.minecraftservices.com")));
    let (status, v) = read(resp, "the chat key service").map_err(|e| e.to_string())?;
    if status != 200 {
        return Err(format!("the chat key service said {status}"));
    }
    let text = |a: &str, b: Option<&str>| -> std::result::Result<String, String> {
        let at = match b {
            Some(b) => &v[a][b],
            None => &v[a],
        };
        at.as_str().map(str::to_owned).ok_or_else(|| format!("the chat keys had no {a}"))
    };
    Ok(ProfileKeys {
        private_key_pem: text("keyPair", Some("privateKey"))?,
        public_key_pem: text("keyPair", Some("publicKey"))?,
        key_signature: text("publicKeySignatureV2", None)?,
        expires_at: text("expiresAt", None)?,
    })
}

/// Tells the session server we're joining; required by online-mode servers.
pub fn join_server(access_token: &str, profile_id: Uuid, server_hash: &str) -> std::result::Result<(), String> {
    let (status, body) = read(
        post_json(
            JOIN_URL,
            &json!({"accessToken": access_token, "selectedProfile": profile_id.simple().to_string(), "serverId": server_hash}),
        ),
        "the session server",
    )
    .map_err(|e| e.to_string())?;
    match status {
        200 | 204 => Ok(()),
        403 => Err(match body["error"].as_str() {
            Some("InsufficientPrivilegesException") => "Multiplayer is turned off for this account in its Xbox privacy settings.".into(),
            Some("UserBannedException") => "This account is banned from multiplayer.".into(),
            _ => "Mojang rejected the join request. Sign in again and retry.".into(),
        }),
        401 => Err("Your session expired. Sign in again and retry.".into()),
        429 => Err("Too many join attempts. Wait a moment and try again.".into()),
        s => Err(format!("The session server answered with HTTP {s}. Try again shortly.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xbox_error_codes() {
        assert_eq!(xerr(&json!({"XErr": 2148916233u64})), AuthError::NoXboxAccount);
        assert_eq!(xerr(&json!({"XErr": 2148916238u64})), AuthError::Child);
        assert_eq!(xerr(&json!({"XErr": 2148916235u64})), AuthError::Region);
        assert_eq!(xerr(&json!({"XErr": 1})), AuthError::Xbox(1));
    }

    #[test]
    fn oauth_errors() {
        assert_eq!(oauth_error(&json!({"error": "expired_token"})), AuthError::Expired);
        assert_eq!(oauth_error(&json!({"error": "authorization_declined"})), AuthError::Declined);
        assert_eq!(oauth_error(&json!({"error": "invalid_grant"})), AuthError::RefreshRejected);
    }

    #[test]
    fn empty_client_id_is_reported_before_any_request() {
        assert_eq!(start_device_code("  ").unwrap_err(), AuthError::NoClientId);
    }
}
