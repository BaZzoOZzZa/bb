use std::path::{Path, PathBuf};

use bb_wire::system::{HealthResponse, SystemVersionQuery, SystemVersionResponse};
use serde::Deserialize;

use crate::endpoint::ServerEndpoint;
use crate::error::ClientError;
use crate::http::BbClient;

pub const DEFAULT_SERVER_URL: &str = "http://127.0.0.1:38886";
pub const SERVER_URL_ENV: &str = "BB_SERVER_URL";
pub const DATA_DIR_ENV: &str = "BB_DATA_DIR";
pub const APP_RUNTIME_FILE_NAME: &str = "bb-app-runtime.json";
pub const MIN_SUPPORTED_SERVER_VERSION: (u64, u64, u64) = (0, 45, 0);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerUrlSource {
    Flag,
    Env,
    RuntimeFile { path: PathBuf, pid: u32 },
    Default,
}

impl ServerUrlSource {
    pub fn label(&self) -> String {
        match self {
            Self::Flag => "--server-url".into(),
            Self::Env => SERVER_URL_ENV.into(),
            Self::RuntimeFile { path, .. } => path.display().to_string(),
            Self::Default => "default".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredServer {
    pub endpoint: ServerEndpoint,
    pub source: ServerUrlSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DiscoveryInput {
    pub flag_url: Option<String>,
    pub env_url: Option<String>,
    pub data_dir: Option<PathBuf>,
}

impl DiscoveryInput {
    pub fn from_env(flag_url: Option<String>) -> Self {
        let home = dirs::home_dir();
        let data_dir = std::env::var(DATA_DIR_ENV)
            .ok()
            .and_then(|raw| expand_data_dir(&raw, home.as_deref()))
            .or_else(|| home.map(|home| home.join(".bb")));
        Self {
            flag_url,
            env_url: std::env::var(SERVER_URL_ENV).ok(),
            data_dir,
        }
    }
}

fn expand_data_dir(raw: &str, home: Option<&Path>) -> Option<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    match (trimmed.strip_prefix('~'), home) {
        (Some(rest), Some(home)) if rest.is_empty() || rest.starts_with('/') => {
            Some(home.join(rest.trim_start_matches('/')))
        }
        _ => Some(PathBuf::from(trimmed)),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppRuntimeFile {
    pub pid: u32,
    pub server_url: String,
    #[serde(default)]
    pub entry_path: String,
    #[serde(default)]
    pub surface: String,
    #[serde(default)]
    pub started_at: String,
    #[serde(default)]
    pub version: String,
}

pub fn read_app_runtime_file(data_dir: &Path) -> Option<AppRuntimeFile> {
    let raw = std::fs::read_to_string(data_dir.join(APP_RUNTIME_FILE_NAME)).ok()?;
    let parsed: AppRuntimeFile = serde_json::from_str(&raw).ok()?;
    (parsed.pid > 0 && !parsed.server_url.trim().is_empty()).then_some(parsed)
}

pub fn process_is_running(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    if cfg!(unix) {
        let proc_root = Path::new("/proc");
        if proc_root.join("self").exists() {
            return proc_root.join(pid.to_string()).exists();
        }
        return std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
    }
    true
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

pub fn discover_server(
    input: &DiscoveryInput,
    is_running: impl Fn(u32) -> bool,
) -> Result<DiscoveredServer, ClientError> {
    if let Some(url) = non_empty(input.flag_url.as_deref()) {
        return Ok(DiscoveredServer {
            endpoint: ServerEndpoint::parse(url)?,
            source: ServerUrlSource::Flag,
        });
    }
    if let Some(url) = non_empty(input.env_url.as_deref()) {
        return Ok(DiscoveredServer {
            endpoint: ServerEndpoint::parse(url)
                .map_err(|error| ClientError::InvalidUrl(format!("{SERVER_URL_ENV}: {error}")))?,
            source: ServerUrlSource::Env,
        });
    }
    if let Some(data_dir) = &input.data_dir
        && let Some(runtime) = read_app_runtime_file(data_dir)
        && is_running(runtime.pid)
    {
        match ServerEndpoint::parse(&runtime.server_url) {
            Ok(endpoint) => {
                return Ok(DiscoveredServer {
                    endpoint,
                    source: ServerUrlSource::RuntimeFile {
                        path: data_dir.join(APP_RUNTIME_FILE_NAME),
                        pid: runtime.pid,
                    },
                });
            }
            Err(error) => {
                tracing::warn!(%error, "ignoring invalid serverUrl in bb app runtime file");
            }
        }
    }
    Ok(DiscoveredServer {
        endpoint: ServerEndpoint::parse(DEFAULT_SERVER_URL)?,
        source: ServerUrlSource::Default,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompatWarning {
    VersionUnavailable(String),
    UnparsableVersion(String),
    ServerTooOld { found: String, minimum: String },
    ServerMoving,
}

impl std::fmt::Display for CompatWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::VersionUnavailable(error) => {
                write!(f, "could not read the server version: {error}")
            }
            Self::UnparsableVersion(version) => {
                write!(f, "unrecognized server version {version}")
            }
            Self::ServerTooOld { found, minimum } => write!(
                f,
                "server version {found} is older than {minimum}; some features may not work"
            ),
            Self::ServerMoving => f.write_str("the server is moving to another machine"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProbeReport {
    pub health: HealthResponse,
    pub version: Option<SystemVersionResponse>,
    pub warnings: Vec<CompatWarning>,
}

pub fn parse_version(raw: &str) -> Option<(u64, u64, u64)> {
    let core = raw.trim().trim_start_matches('v');
    let core = core.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|part| part.parse::<u64>().ok());
    let major = parts.next()??;
    let minor = parts.next().unwrap_or(Some(0))?;
    let patch = parts.next().unwrap_or(Some(0))?;
    Some((major, minor, patch))
}

pub fn version_warnings(version: &SystemVersionResponse) -> Vec<CompatWarning> {
    if version.is_development {
        return Vec::new();
    }
    let (major, minor, patch) = MIN_SUPPORTED_SERVER_VERSION;
    match parse_version(&version.current_version) {
        None => vec![CompatWarning::UnparsableVersion(
            version.current_version.clone(),
        )],
        Some(found) if found < MIN_SUPPORTED_SERVER_VERSION => vec![CompatWarning::ServerTooOld {
            found: version.current_version.clone(),
            minimum: format!("{major}.{minor}.{patch}"),
        }],
        Some(_) => Vec::new(),
    }
}

pub async fn probe_server(client: &BbClient) -> Result<ProbeReport, ClientError> {
    let health = client.health().await?;
    if !health.ok {
        return Err(ClientError::InvalidUrl(format!(
            "{} did not report ok from /health",
            client.endpoint()
        )));
    }
    let mut warnings = Vec::new();
    if health
        .server_move
        .as_ref()
        .is_some_and(|server_move| server_move.state.is_known())
    {
        warnings.push(CompatWarning::ServerMoving);
    }
    let version = match client.system_version(&SystemVersionQuery::default()).await {
        Ok(version) => {
            warnings.extend(version_warnings(&version));
            Some(version)
        }
        Err(error) => {
            warnings.push(CompatWarning::VersionUnavailable(error.to_string()));
            None
        }
    };
    Ok(ProbeReport {
        health,
        version,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_runtime(dir: &Path, pid: u32, url: &str) {
        std::fs::write(
            dir.join(APP_RUNTIME_FILE_NAME),
            format!(
                r#"{{"entryPath":"/x/bb-app.js","pid":{pid},"surface":"desktop","serverUrl":"{url}","startedAt":"2026-01-01T00:00:00Z","version":"0.45.0"}}"#
            ),
        )
        .unwrap();
    }

    #[test]
    fn precedence_is_flag_env_runtime_default() {
        let dir = tempfile::tempdir().unwrap();
        write_runtime(dir.path(), 4242, "http://127.0.0.1:40000");
        let mut input = DiscoveryInput {
            flag_url: Some("http://flag:1".into()),
            env_url: Some("http://env:2".into()),
            data_dir: Some(dir.path().to_path_buf()),
        };
        let alive = |pid: u32| pid == 4242;
        let found = discover_server(&input, alive).unwrap();
        assert_eq!(found.source, ServerUrlSource::Flag);
        assert_eq!(found.endpoint.to_string(), "http://flag:1");

        input.flag_url = Some("  ".into());
        let found = discover_server(&input, alive).unwrap();
        assert_eq!(found.source, ServerUrlSource::Env);

        input.env_url = None;
        let found = discover_server(&input, alive).unwrap();
        assert_eq!(found.endpoint.to_string(), "http://127.0.0.1:40000");
        assert!(matches!(
            found.source,
            ServerUrlSource::RuntimeFile { pid: 4242, .. }
        ));

        let found = discover_server(&input, |_| false).unwrap();
        assert_eq!(found.source, ServerUrlSource::Default);
        assert_eq!(found.endpoint.to_string(), DEFAULT_SERVER_URL);
    }

    #[test]
    fn invalid_runtime_file_falls_back_and_invalid_flag_fails() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(APP_RUNTIME_FILE_NAME), "{not json").unwrap();
        let input = DiscoveryInput {
            data_dir: Some(dir.path().to_path_buf()),
            ..DiscoveryInput::default()
        };
        assert_eq!(
            discover_server(&input, |_| true).unwrap().source,
            ServerUrlSource::Default
        );
        write_runtime(dir.path(), 7, "ftp://nope");
        assert_eq!(
            discover_server(&input, |_| true).unwrap().source,
            ServerUrlSource::Default
        );
        let bad = DiscoveryInput {
            flag_url: Some("ftp://x".into()),
            ..DiscoveryInput::default()
        };
        assert!(discover_server(&bad, |_| true).is_err());
    }

    #[test]
    fn current_process_is_running() {
        assert!(process_is_running(std::process::id()));
        assert!(!process_is_running(0));
    }

    #[test]
    fn data_dir_expands_home() {
        let home = Path::new("/home/u");
        assert_eq!(
            expand_data_dir("~/custom", Some(home)),
            Some(PathBuf::from("/home/u/custom"))
        );
        assert_eq!(expand_data_dir(" ", Some(home)), None);
        assert_eq!(
            expand_data_dir("/abs", Some(home)),
            Some(PathBuf::from("/abs"))
        );
    }

    #[test]
    fn versions_compare_softly() {
        assert_eq!(parse_version("v0.45.1-beta.2"), Some((0, 45, 1)));
        assert_eq!(parse_version("1.2"), Some((1, 2, 0)));
        assert_eq!(parse_version("dev"), None);
        let mut version = SystemVersionResponse {
            current_version: "0.44.9".into(),
            ..SystemVersionResponse::default()
        };
        assert!(matches!(
            version_warnings(&version).as_slice(),
            [CompatWarning::ServerTooOld { .. }]
        ));
        version.current_version = "0.46.0".into();
        assert!(version_warnings(&version).is_empty());
        version.current_version = "nightly".into();
        version.is_development = true;
        assert!(version_warnings(&version).is_empty());
    }
}
