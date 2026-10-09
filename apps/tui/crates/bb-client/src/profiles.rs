use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::auth::Auth;
use crate::connect::{
    ConnectCredential, ConnectError, ConnectSession, PairingInput, connect_http_client,
    redeem_machine_credential, server_url_for_handle,
};
use crate::endpoint::ServerEndpoint;
use crate::error::ClientError;
use crate::http::BbClient;

pub const CONFIG_DIR_NAME: &str = "bb-tui";
pub const PROFILES_FILE_NAME: &str = "servers.json";
pub const CREDENTIALS_FILE_NAME: &str = "credentials.json";
pub const KEYRING_SERVICE: &str = "bb-tui";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum ServerProfile {
    Local {
        id: String,
        name: String,
        url: String,
    },
    Connect {
        id: String,
        name: String,
        server_url: String,
        handle: String,
        apex_url: String,
    },
}

impl ServerProfile {
    pub fn local(name: impl Into<String>, url: impl Into<String>) -> Self {
        let url = url.into();
        Self::Local {
            id: format!("local:{url}"),
            name: name.into(),
            url,
        }
    }

    pub fn connect(credential: &ConnectCredential, apex_url: impl Into<String>) -> Self {
        Self::Connect {
            id: format!("connect:{}", credential.handle),
            name: credential.handle.clone(),
            server_url: credential.server_url.clone(),
            handle: credential.handle.clone(),
            apex_url: apex_url.into(),
        }
    }

    pub fn id(&self) -> &str {
        match self {
            Self::Local { id, .. } | Self::Connect { id, .. } => id,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Local { name, .. } | Self::Connect { name, .. } => name,
        }
    }

    pub fn url(&self) -> &str {
        match self {
            Self::Local { url, .. } => url,
            Self::Connect { server_url, .. } => server_url,
        }
    }

    pub fn is_connect(&self) -> bool {
        matches!(self, Self::Connect { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileList {
    pub active: Option<String>,
    pub profiles: Vec<ServerProfile>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawProfileList {
    active: Option<String>,
    profiles: Vec<serde_json::Value>,
}

impl ProfileList {
    pub fn parse(raw: &str) -> Self {
        let parsed: RawProfileList = serde_json::from_str(raw).unwrap_or_default();
        let profiles: Vec<ServerProfile> = parsed
            .profiles
            .into_iter()
            .filter_map(|value| serde_json::from_value(value).ok())
            .collect();
        let active = parsed
            .active
            .filter(|id| profiles.iter().any(|profile| profile.id() == id));
        Self { active, profiles }
    }

    pub fn get(&self, id: &str) -> Option<&ServerProfile> {
        self.profiles.iter().find(|profile| profile.id() == id)
    }

    pub fn active(&self) -> Option<&ServerProfile> {
        self.get(self.active.as_deref()?)
    }

    pub fn upsert(&mut self, profile: ServerProfile) {
        match self
            .profiles
            .iter_mut()
            .find(|existing| existing.id() == profile.id())
        {
            Some(existing) => *existing = profile,
            None => self.profiles.push(profile),
        }
    }

    pub fn remove(&mut self, id: &str) -> Option<ServerProfile> {
        let index = self
            .profiles
            .iter()
            .position(|profile| profile.id() == id)?;
        if self.active.as_deref() == Some(id) {
            self.active = None;
        }
        Some(self.profiles.remove(index))
    }

    pub fn set_active(&mut self, id: &str) -> bool {
        let known = self.get(id).is_some();
        if known {
            self.active = Some(id.to_owned());
        }
        known
    }
}

pub fn default_config_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| dirs::home_dir().map(|home| home.join(".config")))
        .map(|base| base.join(CONFIG_DIR_NAME))
}

fn write_private(path: &Path, contents: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no parent"))?;
    std::fs::create_dir_all(parent)?;
    let temp = path.with_extension("tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    {
        let mut file = options.open(&temp)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        file.write_all(contents)?;
        file.sync_all()?;
    }
    std::fs::rename(&temp, path)
}

#[derive(Debug, Clone)]
pub struct ProfileStore {
    path: PathBuf,
}

impl ProfileStore {
    pub fn new(config_dir: &Path) -> Self {
        Self {
            path: config_dir.join(PROFILES_FILE_NAME),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> io::Result<ProfileList> {
        match std::fs::read_to_string(&self.path) {
            Ok(raw) => Ok(ProfileList::parse(&raw)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(ProfileList::default()),
            Err(error) => Err(error),
        }
    }

    pub fn save(&self, list: &ProfileList) -> io::Result<()> {
        let mut bytes = serde_json::to_vec_pretty(list).map_err(io::Error::other)?;
        bytes.push(b'\n');
        write_private(&self.path, &bytes)
    }
}

pub trait CredentialStore: Send + Sync {
    fn load(&self, profile_id: &str) -> io::Result<Option<String>>;
    fn store(&self, profile_id: &str, credential: &str) -> io::Result<()>;
    fn delete(&self, profile_id: &str) -> io::Result<()>;
    fn describe(&self) -> String;
}

#[derive(Debug, Clone)]
pub struct FileCredentialStore {
    path: PathBuf,
}

#[derive(Serialize, Deserialize, Default)]
struct CredentialFile {
    #[serde(default)]
    credentials: BTreeMap<String, String>,
}

impl FileCredentialStore {
    pub fn new(config_dir: &Path) -> Self {
        Self {
            path: config_dir.join(CREDENTIALS_FILE_NAME),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn read(&self) -> io::Result<CredentialFile> {
        match std::fs::read_to_string(&self.path) {
            Ok(raw) => Ok(serde_json::from_str(&raw).unwrap_or_default()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(CredentialFile::default()),
            Err(error) => Err(error),
        }
    }

    fn write(&self, file: &CredentialFile) -> io::Result<()> {
        let bytes = serde_json::to_vec_pretty(file).map_err(io::Error::other)?;
        write_private(&self.path, &bytes)
    }
}

impl CredentialStore for FileCredentialStore {
    fn load(&self, profile_id: &str) -> io::Result<Option<String>> {
        Ok(self.read()?.credentials.remove(profile_id))
    }

    fn store(&self, profile_id: &str, credential: &str) -> io::Result<()> {
        let mut file = self.read()?;
        file.credentials
            .insert(profile_id.to_owned(), credential.to_owned());
        self.write(&file)
    }

    fn delete(&self, profile_id: &str) -> io::Result<()> {
        let mut file = self.read()?;
        if file.credentials.remove(profile_id).is_some() {
            self.write(&file)?;
        }
        Ok(())
    }

    fn describe(&self) -> String {
        self.path.display().to_string()
    }
}

#[cfg(feature = "keyring")]
pub struct KeyringCredentialStore {
    fallback: FileCredentialStore,
}

#[cfg(feature = "keyring")]
impl KeyringCredentialStore {
    pub fn available() -> bool {
        keyring::Entry::store_status().is_ok()
    }

    pub fn new(fallback: FileCredentialStore) -> Self {
        Self { fallback }
    }

    fn entry(profile_id: &str) -> Option<keyring::Entry> {
        keyring::Entry::new(KEYRING_SERVICE, profile_id).ok()
    }
}

#[cfg(feature = "keyring")]
impl CredentialStore for KeyringCredentialStore {
    fn load(&self, profile_id: &str) -> io::Result<Option<String>> {
        if let Some(entry) = Self::entry(profile_id)
            && let Ok(secret) = entry.get_password()
        {
            return Ok(Some(secret));
        }
        self.fallback.load(profile_id)
    }

    fn store(&self, profile_id: &str, credential: &str) -> io::Result<()> {
        if let Some(entry) = Self::entry(profile_id)
            && entry.set_password(credential).is_ok()
        {
            return self.fallback.delete(profile_id);
        }
        self.fallback.store(profile_id, credential)
    }

    fn delete(&self, profile_id: &str) -> io::Result<()> {
        if let Some(entry) = Self::entry(profile_id) {
            let _ = entry.delete_credential();
        }
        self.fallback.delete(profile_id)
    }

    fn describe(&self) -> String {
        format!("OS keyring (fallback {})", self.fallback.describe())
    }
}

pub fn default_credential_store(config_dir: &Path) -> Box<dyn CredentialStore> {
    let file = FileCredentialStore::new(config_dir);
    #[cfg(feature = "keyring")]
    if KeyringCredentialStore::available() {
        return Box::new(KeyringCredentialStore::new(file));
    }
    Box::new(file)
}

pub fn connect_credential(
    profile: &ServerProfile,
    store: &dyn CredentialStore,
) -> Result<Option<ConnectCredential>, ClientError> {
    let ServerProfile::Connect {
        id,
        server_url,
        handle,
        ..
    } = profile
    else {
        return Ok(None);
    };
    let credential = store
        .load(id)?
        .ok_or(ClientError::Connect(ConnectError::NotPaired))?;
    Ok(Some(ConnectCredential {
        server_url: server_url.clone(),
        handle: handle.clone(),
        credential,
    }))
}

pub fn client_for_profile(
    profile: &ServerProfile,
    store: &dyn CredentialStore,
) -> Result<BbClient, ClientError> {
    let endpoint = ServerEndpoint::parse(profile.url())?;
    let auth = match connect_credential(profile, store)? {
        Some(credential) => Auth::Connect(std::sync::Arc::new(ConnectSession::new(credential))),
        None => Auth::None,
    };
    BbClient::new(endpoint, auth)
}

pub async fn pair_connect_profile(
    input: &PairingInput,
    device_name: Option<&str>,
    store: &dyn CredentialStore,
) -> Result<ServerProfile, ClientError> {
    let apex_url = input.apex_url();
    let credential =
        redeem_machine_credential(&connect_http_client(), &apex_url, &input.code, device_name)
            .await?;
    let profile = ServerProfile::connect(&credential, apex_url);
    store.store(profile.id(), &credential.credential)?;
    Ok(profile)
}

pub fn connect_profile_for_handle(
    apex_url: &str,
    handle: &str,
    name: &str,
) -> Option<ServerProfile> {
    Some(ServerProfile::Connect {
        id: format!("connect:{handle}"),
        name: name.to_owned(),
        server_url: server_url_for_handle(apex_url, handle)?,
        handle: handle.to_owned(),
        apex_url: apex_url.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_list_round_trips_and_skips_unknown_entries() {
        let raw = r#"{
            "active": "connect:bee",
            "profiles": [
                {"kind":"local","id":"local:http://127.0.0.1:38886","name":"This machine","url":"http://127.0.0.1:38886"},
                {"kind":"connect","id":"connect:bee","name":"bee","serverUrl":"https://bee.getbb.app","handle":"bee","apexUrl":"https://getbb.app","extra":1},
                {"kind":"ssh","id":"x"}
            ],
            "future": true
        }"#;
        let list = ProfileList::parse(raw);
        assert_eq!(list.profiles.len(), 2);
        assert_eq!(list.active().map(ServerProfile::name), Some("bee"));
        let encoded = serde_json::to_string(&list).unwrap();
        assert_eq!(ProfileList::parse(&encoded), list);
        assert_eq!(ProfileList::parse("garbage"), ProfileList::default());
    }

    #[test]
    fn profile_list_mutations() {
        let mut list = ProfileList::default();
        list.upsert(ServerProfile::local("a", "http://a:1"));
        list.upsert(ServerProfile::local("renamed", "http://a:1"));
        assert_eq!(list.profiles.len(), 1);
        assert_eq!(list.profiles[0].name(), "renamed");
        assert!(list.set_active("local:http://a:1"));
        assert!(!list.set_active("missing"));
        list.remove("local:http://a:1");
        assert!(list.active.is_none());
        assert!(list.profiles.is_empty());
    }

    #[test]
    fn credentials_file_is_private_and_profiles_hold_no_secret() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("cfg");
        let store = FileCredentialStore::new(&config);
        assert_eq!(store.load("connect:bee").unwrap(), None);
        store.store("connect:bee", "bbcm_secret").unwrap();
        assert_eq!(
            store.load("connect:bee").unwrap().as_deref(),
            Some("bbcm_secret")
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(store.path())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let credential = ConnectCredential {
            server_url: "https://bee.getbb.app".into(),
            handle: "bee".into(),
            credential: "bbcm_secret".into(),
        };
        let profile = ServerProfile::connect(&credential, "https://getbb.app");
        let profiles = ProfileStore::new(&config);
        let mut list = ProfileList::default();
        list.upsert(profile.clone());
        profiles.save(&list).unwrap();
        let saved = std::fs::read_to_string(profiles.path()).unwrap();
        assert!(!saved.contains("bbcm_secret"));
        assert_eq!(profiles.load().unwrap(), list);

        let loaded = connect_credential(&profile, &store).unwrap().unwrap();
        assert_eq!(loaded, credential);
        store.delete("connect:bee").unwrap();
        assert!(matches!(
            connect_credential(&profile, &store),
            Err(ClientError::Connect(ConnectError::NotPaired))
        ));
        let local = ServerProfile::local("l", "http://127.0.0.1:1");
        assert!(connect_credential(&local, &store).unwrap().is_none());
    }

    #[test]
    fn connect_profile_from_handle() {
        let profile = connect_profile_for_handle("https://getbb.app", "wasp", "Wasp").unwrap();
        assert_eq!(profile.url(), "https://wasp.getbb.app");
        assert!(profile.is_connect());
    }
}
