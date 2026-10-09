use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use url::Url;

pub const DEFAULT_CONNECT_APEX_URL: &str = "https://getbb.app";
pub const MACHINE_CREDENTIAL_HEADER: &str = "x-bb-connect-machine";
pub const SESSION_RENEWAL_LEAD: Duration = Duration::from_secs(5 * 60);
const CONNECT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConnectError {
    #[error("this pairing code was already used")]
    AlreadyUsed,
    #[error("this pairing code has expired")]
    Expired,
    #[error("invalid pairing code: {0}")]
    InvalidCode(String),
    #[error("the bb account has no free machine slots; revoke an unused device first")]
    MachineLimit,
    #[error("bb connect is not paired for this server")]
    NotPaired,
    #[error("bb connect rejected the machine credential: {0}")]
    Unauthorized(String),
    #[error("bb connect network error: {0}")]
    Network(String),
    #[error("bb connect returned an invalid response: {0}")]
    InvalidResponse(String),
}

impl ConnectError {
    pub fn requires_pairing(&self) -> bool {
        matches!(self, Self::Unauthorized(_) | Self::NotPaired)
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectCredential {
    pub server_url: String,
    pub handle: String,
    pub credential: String,
}

impl fmt::Debug for ConnectCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectCredential")
            .field("server_url", &self.server_url)
            .field("handle", &self.handle)
            .field("credential", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionCookie {
    pub domain: String,
    pub expires_at: i64,
    pub name: String,
    pub value: String,
}

impl fmt::Debug for SessionCookie {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionCookie")
            .field("domain", &self.domain)
            .field("expires_at", &self.expires_at)
            .field("name", &self.name)
            .field("value", &"<redacted>")
            .finish()
    }
}

impl SessionCookie {
    pub fn header_value(&self) -> String {
        format!("{}={}", self.name, self.value)
    }

    pub fn renew_at_ms(&self) -> i64 {
        self.expires_at - SESSION_RENEWAL_LEAD.as_millis() as i64
    }

    pub fn needs_renewal(&self, now_ms: i64) -> bool {
        now_ms >= self.renew_at_ms()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DesktopSession {
    pub cookie: SessionCookie,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AccountServer {
    pub handle: String,
    pub name: String,
    pub live: bool,
    #[serde(skip)]
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountServers {
    pub servers: Vec<AccountServer>,
    pub self_handle: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingInput {
    pub code: String,
    pub server_url: Option<String>,
    pub apex_url: Option<String>,
    pub expires_at: Option<i64>,
}

impl PairingInput {
    pub fn apex_url(&self) -> String {
        self.apex_url
            .clone()
            .or_else(|| self.server_url.as_deref().and_then(derive_connect_base_url))
            .unwrap_or_else(|| DEFAULT_CONNECT_APEX_URL.to_owned())
    }
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or_default()
}

pub fn connect_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(CONNECT_REQUEST_TIMEOUT)
        .user_agent(concat!("bb-tui/", env!("CARGO_PKG_VERSION")))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

fn http_origin(raw: &str) -> Option<String> {
    let url = Url::parse(raw.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    url.host_str()?;
    Some(url.origin().ascii_serialization())
}

pub fn derive_connect_base_url(server_url: &str) -> Option<String> {
    let url = Url::parse(server_url).ok()?;
    let host = url.host_str()?;
    let apex_host = host.split_once('.').map_or(host, |(_, rest)| rest);
    let port = url
        .port()
        .map(|port| format!(":{port}"))
        .unwrap_or_default();
    Some(format!("{}://{apex_host}{port}", url.scheme()))
}

pub fn server_url_for_handle(base_url: &str, handle: &str) -> Option<String> {
    let url = Url::parse(base_url).ok()?;
    let host = url.host_str()?;
    let port = url
        .port()
        .map(|port| format!(":{port}"))
        .unwrap_or_default();
    Some(format!("{}://{handle}.{host}{port}", url.scheme()))
}

pub fn handle_for_apex(server_url: &str, apex_url: &str) -> Option<String> {
    let server = Url::parse(server_url).ok()?;
    let apex = Url::parse(apex_url).ok()?;
    if server.scheme() != apex.scheme() || server.port() != apex.port() {
        return None;
    }
    let (label, rest) = server.host_str()?.split_once('.')?;
    (!label.is_empty() && Some(rest) == apex.host_str()).then(|| label.to_owned())
}

pub fn normalize_connect_code(raw: &str) -> String {
    raw.trim().to_uppercase()
}

pub fn is_valid_connect_code(code: &str) -> bool {
    let bytes = code.as_bytes();
    (4..=64).contains(&bytes.len())
        && (bytes[0].is_ascii_uppercase() || bytes[0].is_ascii_digit())
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || *byte == b'-')
}

fn json_epoch_ms(value: Option<&serde_json::Value>) -> Option<i64> {
    match value? {
        serde_json::Value::Number(number) => number
            .as_f64()
            .filter(|ms| ms.is_finite() && *ms > 0.0)
            .map(|ms| ms as i64),
        serde_json::Value::String(text) => text.trim().parse::<i64>().ok().filter(|ms| *ms > 0),
        _ => None,
    }
}

fn pairing_from_fields(
    code: Option<&str>,
    server_url: Option<&str>,
    apex: Option<&str>,
    expires_at: Option<i64>,
) -> Option<PairingInput> {
    let code = normalize_connect_code(code?);
    is_valid_connect_code(&code).then(|| PairingInput {
        code,
        server_url: server_url.and_then(http_origin),
        apex_url: apex.and_then(http_origin),
        expires_at,
    })
}

pub fn parse_pairing_input(raw: &str) -> Option<PairingInput> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with('{') {
        let record: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(trimmed).ok()?;
        let text = |key: &str| record.get(key).and_then(serde_json::Value::as_str);
        return pairing_from_fields(
            text("code"),
            text("serverUrl"),
            text("apex").or_else(|| text("apexUrl")),
            json_epoch_ms(record.get("expiresAt")),
        );
    }
    if let Some((scheme, _)) = trimmed.split_once("://")
        && !scheme.is_empty()
        && scheme
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '.' | '-'))
    {
        let url = Url::parse(trimmed).ok()?;
        let params: Vec<(String, String)> = url.query_pairs().into_owned().collect();
        let param = |key: &str| {
            params
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.as_str())
        };
        return pairing_from_fields(
            param("code"),
            param("serverUrl").or_else(|| param("server")),
            param("apex").or_else(|| param("apexUrl")),
            param("expiresAt").and_then(|value| value.parse().ok()),
        );
    }
    pairing_from_fields(Some(trimmed), None, None, None)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RedeemMachineRequest<'a> {
    code: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    device_name: Option<&'a str>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RedeemMachineResponse {
    credential: String,
    #[serde(default)]
    machine_id: String,
    server_url: Option<String>,
}

#[derive(Deserialize)]
struct RedeemMachineErrorBody {
    error: String,
}

fn redeem_error_for_status(status: u16, wire_error: &str) -> ConnectError {
    let message = if wire_error.is_empty() {
        format!("machine redeem failed ({status})")
    } else {
        format!("machine redeem failed ({status}): {wire_error}")
    };
    match wire_error {
        "machine-limit" => ConnectError::MachineLimit,
        "already-used" => ConnectError::AlreadyUsed,
        "expired" => ConnectError::Expired,
        _ if status == 409 => ConnectError::AlreadyUsed,
        _ if status == 410 => ConnectError::Expired,
        _ if status >= 500 => ConnectError::Network(message),
        _ => ConnectError::InvalidCode(message),
    }
}

fn trim_slash(url: &str) -> &str {
    url.strip_suffix('/').unwrap_or(url)
}

pub async fn redeem_machine_credential(
    http: &reqwest::Client,
    apex_url: &str,
    code: &str,
    device_name: Option<&str>,
) -> Result<ConnectCredential, ConnectError> {
    let url = format!("{}/api/connect/redeem-machine", trim_slash(apex_url));
    let response = http
        .post(&url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(&RedeemMachineRequest { code, device_name })
        .send()
        .await
        .map_err(|error| ConnectError::Network(error.to_string()))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| ConnectError::Network(error.to_string()))?;
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&body) else {
        return Err(ConnectError::InvalidResponse(
            "machine redeem returned non-JSON".into(),
        ));
    };
    if !status.is_success() {
        let wire_error = serde_json::from_value::<RedeemMachineErrorBody>(value)
            .map(|body| body.error)
            .unwrap_or_default();
        return Err(redeem_error_for_status(status.as_u16(), &wire_error));
    }
    let parsed: RedeemMachineResponse = serde_json::from_value(value).map_err(|_| {
        ConnectError::InvalidResponse("machine redeem response failed schema validation".into())
    })?;
    if parsed.credential.is_empty() {
        return Err(ConnectError::InvalidResponse(
            "machine redeem returned an empty credential".into(),
        ));
    }
    let Some(server_url) = parsed.server_url else {
        return Err(ConnectError::InvalidResponse(
            "machine redeem returned no server URL".into(),
        ));
    };
    let handle = handle_for_apex(&server_url, apex_url).ok_or_else(|| {
        ConnectError::InvalidResponse(format!(
            "machine redeem returned a server URL outside {apex_url}"
        ))
    })?;
    tracing::debug!(machine_id = %parsed.machine_id, %handle, "redeemed bb connect machine code");
    Ok(ConnectCredential {
        server_url,
        handle,
        credential: parsed.credential,
    })
}

async fn credential_request(
    http: &reqwest::Client,
    method: reqwest::Method,
    credential: &ConnectCredential,
    path: &str,
    label: &str,
) -> Result<serde_json::Value, ConnectError> {
    let url = format!("{}{path}", trim_slash(&credential.server_url));
    let response = http
        .request(method, &url)
        .header(MACHINE_CREDENTIAL_HEADER, &credential.credential)
        .send()
        .await
        .map_err(|error| ConnectError::Network(error.to_string()))?;
    let status = response.status().as_u16();
    if status == 401 || status == 403 {
        return Err(ConnectError::Unauthorized(format!(
            "{label} failed ({status}): not authorized"
        )));
    }
    if !(200..300).contains(&status) {
        return Err(ConnectError::Network(format!("{label} failed ({status})")));
    }
    response
        .json()
        .await
        .map_err(|_| ConnectError::InvalidResponse(format!("{label} returned non-JSON")))
}

pub async fn fetch_desktop_session(
    http: &reqwest::Client,
    credential: &ConnectCredential,
) -> Result<DesktopSession, ConnectError> {
    let value = credential_request(
        http,
        reqwest::Method::POST,
        credential,
        "/api/connect/desktop-session",
        "desktop session",
    )
    .await?;
    let session: DesktopSession = serde_json::from_value(value).map_err(|_| {
        ConnectError::InvalidResponse("desktop session response failed schema validation".into())
    })?;
    let cookie = &session.cookie;
    if cookie.name.is_empty() || cookie.value.is_empty() || cookie.expires_at <= 0 {
        return Err(ConnectError::InvalidResponse(
            "desktop session response failed schema validation".into(),
        ));
    }
    Ok(session)
}

#[derive(Deserialize)]
struct AccountServersBody {
    servers: Vec<AccountServer>,
}

pub async fn list_account_servers(
    http: &reqwest::Client,
    credential: &ConnectCredential,
) -> Result<AccountServers, ConnectError> {
    let value = credential_request(
        http,
        reqwest::Method::GET,
        credential,
        "/api/connect/servers",
        "list servers",
    )
    .await?;
    let body: AccountServersBody = serde_json::from_value(value).map_err(|_| {
        ConnectError::InvalidResponse("list servers response failed schema validation".into())
    })?;
    let base = derive_connect_base_url(&credential.server_url).ok_or_else(|| {
        ConnectError::InvalidResponse(format!("invalid server url {}", credential.server_url))
    })?;
    let servers = body
        .servers
        .into_iter()
        .map(|mut server| {
            server.url = server_url_for_handle(&base, &server.handle).unwrap_or_default();
            server
        })
        .collect();
    Ok(AccountServers {
        servers,
        self_handle: credential.handle.clone(),
    })
}

pub struct ConnectSession {
    credential: ConnectCredential,
    http: reqwest::Client,
    cookie: Mutex<Option<SessionCookie>>,
}

impl fmt::Debug for ConnectSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectSession")
            .field("credential", &self.credential)
            .finish_non_exhaustive()
    }
}

impl ConnectSession {
    pub fn new(credential: ConnectCredential) -> Self {
        Self::with_http(credential, connect_http_client())
    }

    pub fn with_http(credential: ConnectCredential, http: reqwest::Client) -> Self {
        Self {
            credential,
            http,
            cookie: Mutex::new(None),
        }
    }

    pub fn credential(&self) -> &ConnectCredential {
        &self.credential
    }

    pub fn machine_credential(&self) -> &str {
        &self.credential.credential
    }

    pub async fn cookie(&self) -> Result<SessionCookie, ConnectError> {
        let mut cached = self.cookie.lock().await;
        if let Some(cookie) = cached.as_ref()
            && !cookie.needs_renewal(now_ms())
        {
            return Ok(cookie.clone());
        }
        let session = fetch_desktop_session(&self.http, &self.credential).await?;
        *cached = Some(session.cookie.clone());
        Ok(session.cookie)
    }

    pub async fn cookie_header(&self) -> Result<String, ConnectError> {
        Ok(self.cookie().await?.header_value())
    }

    pub async fn invalidate(&self) {
        self.cookie.lock().await.take();
    }

    pub async fn renew_in(&self) -> Option<Duration> {
        let cached = self.cookie.lock().await;
        let renew_at = cached.as_ref()?.renew_at_ms();
        Some(Duration::from_millis(
            renew_at.saturating_sub(now_ms()).max(0) as u64,
        ))
    }

    pub async fn list_servers(&self) -> Result<AccountServers, ConnectError> {
        list_account_servers(&self.http, &self.credential).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_apex_and_handles() {
        assert_eq!(
            derive_connect_base_url("https://bee.getbb.app/x").as_deref(),
            Some("https://getbb.app")
        );
        assert_eq!(
            server_url_for_handle("http://connect.localhost:8787", "bee").as_deref(),
            Some("http://bee.connect.localhost:8787")
        );
        assert_eq!(
            handle_for_apex("https://bee.getbb.app", "https://getbb.app").as_deref(),
            Some("bee")
        );
        assert_eq!(
            handle_for_apex("https://bee.evil.app", "https://getbb.app"),
            None
        );
        assert_eq!(
            handle_for_apex("http://bee.getbb.app", "https://getbb.app"),
            None
        );
        assert_eq!(
            handle_for_apex("https://a.bee.getbb.app", "https://getbb.app"),
            None
        );
    }

    #[test]
    fn parses_pairing_inputs() {
        let plain = parse_pairing_input(" abcd-1234 ").unwrap();
        assert_eq!(plain.code, "ABCD-1234");
        assert_eq!(plain.apex_url(), DEFAULT_CONNECT_APEX_URL);

        let json = parse_pairing_input(
            r#"{"code":"wxyz-9876","serverUrl":"https://bee.getbb.app/","apex":"https://getbb.app","expiresAt":1700000000000}"#,
        )
        .unwrap();
        assert_eq!(json.code, "WXYZ-9876");
        assert_eq!(json.server_url.as_deref(), Some("https://bee.getbb.app"));
        assert_eq!(json.expires_at, Some(1_700_000_000_000));

        let link =
            parse_pairing_input("bb://connect?code=QQQQ&server=https://bee.example.dev").unwrap();
        assert_eq!(link.code, "QQQQ");
        assert_eq!(link.apex_url(), "https://example.dev");

        assert!(parse_pairing_input("no").is_none());
        assert!(parse_pairing_input("has space").is_none());
        assert!(parse_pairing_input("-ABC").is_none());
        assert!(parse_pairing_input("{\"serverUrl\":\"https://x\"}").is_none());
    }

    #[test]
    fn maps_redeem_errors() {
        assert_eq!(
            redeem_error_for_status(403, "machine-limit"),
            ConnectError::MachineLimit
        );
        assert_eq!(redeem_error_for_status(409, ""), ConnectError::AlreadyUsed);
        assert_eq!(redeem_error_for_status(410, ""), ConnectError::Expired);
        assert!(matches!(
            redeem_error_for_status(502, ""),
            ConnectError::Network(_)
        ));
        assert!(matches!(
            redeem_error_for_status(400, "bad"),
            ConnectError::InvalidCode(_)
        ));
    }

    #[test]
    fn cookie_renewal_window_and_redaction() {
        let cookie = SessionCookie {
            domain: ".getbb.app".into(),
            expires_at: 10 * 60 * 1000,
            name: "bb-desktop".into(),
            value: "secret".into(),
        };
        assert!(!cookie.needs_renewal(4 * 60 * 1000));
        assert!(cookie.needs_renewal(5 * 60 * 1000));
        assert_eq!(cookie.header_value(), "bb-desktop=secret");
        assert!(!format!("{cookie:?}").contains("secret"));
        let credential = ConnectCredential {
            server_url: "https://bee.getbb.app".into(),
            handle: "bee".into(),
            credential: "bbcm_secret".into(),
        };
        assert!(!format!("{credential:?}").contains("bbcm_secret"));
    }
}
