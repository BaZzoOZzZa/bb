use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use bb_client::connect::parse_pairing_input;
use bb_client::profiles::{
    client_for_profile, default_config_dir, default_credential_store, pair_connect_profile,
};
use bb_client::{
    BbClient, DiscoveryInput, ProfileList, ProfileStore, ServerProfile, discover_server,
    discovery::process_is_running,
};
use clap::{Parser, Subcommand};

use crate::app::ServerInfo;
use crate::composer::SubmitKey;

fn parse_submit_key(value: &str) -> Result<SubmitKey, String> {
    SubmitKey::parse(value)
        .ok_or_else(|| format!("expected `enter` or `ctrl-enter`, got `{value}`"))
}

#[derive(Debug, Parser)]
#[command(name = "bb-tui", version, about = "Terminal client for bb")]
pub struct Args {
    #[arg(long, conflicts_with = "profile")]
    pub server_url: Option<String>,
    #[arg(long, env = "BB_TUI_PROFILE")]
    pub profile: Option<String>,
    #[arg(long, env = "BB_TUI_CONFIG_DIR", hide = true)]
    pub config_dir: Option<PathBuf>,
    #[arg(
        long,
        env = "BB_TUI_READ_ONLY",
        help = "Never send changes to the server (no mark-read, rename, archive, pin)"
    )]
    pub read_only: bool,
    #[arg(
        long,
        env = "BB_TUI_SUBMIT_KEY",
        default_value = "enter",
        value_parser = parse_submit_key,
        help = "Composer send key: `enter` (Shift/Alt+Enter or Ctrl+J for a new line) or `ctrl-enter` (Enter inserts a new line, Alt/Ctrl+Enter sends)"
    )]
    pub submit_key: SubmitKey,
    #[command(subcommand)]
    pub command: Option<TopCommand>,
}

#[derive(Debug, Subcommand)]
pub enum TopCommand {
    #[command(subcommand, about = "Manage bb connect profiles")]
    Connect(ConnectCommand),
}

#[derive(Debug, Subcommand)]
pub enum ConnectCommand {
    #[command(about = "Redeem a machine code from `bb connect machine-code` and save a profile")]
    Pair {
        code: String,
        #[arg(long, help = "Display name for the saved profile")]
        name: Option<String>,
        #[arg(long, help = "Device name reported to bb connect")]
        device_name: Option<String>,
        #[arg(long, help = "Do not make the new profile the active one")]
        no_activate: bool,
    },
    #[command(about = "List saved server profiles")]
    List,
}

pub struct Connection {
    pub client: BbClient,
    pub info: ServerInfo,
    pub source: String,
}

pub fn config_dir(args: &Args) -> Result<PathBuf> {
    args.config_dir
        .clone()
        .or_else(default_config_dir)
        .ok_or_else(|| anyhow!("could not determine the config directory"))
}

pub fn find_profile<'a>(list: &'a ProfileList, query: &str) -> Option<&'a ServerProfile> {
    let query = query.trim();
    list.get(query)
        .or_else(|| list.profiles.iter().find(|profile| profile.name() == query))
        .or_else(|| {
            list.profiles.iter().find(|profile| match profile {
                ServerProfile::Connect { handle, .. } => handle == query,
                ServerProfile::Local { url, .. } => url == query,
            })
        })
}

pub fn resolve_connection(args: &Args) -> Result<Connection> {
    if let Some(query) = &args.profile {
        let dir = config_dir(args)?;
        let list = ProfileStore::new(&dir).load()?;
        let profile = find_profile(&list, query).ok_or_else(|| {
            anyhow!("no saved profile named {query:?}; see `bb-tui connect list`")
        })?;
        let store = default_credential_store(&dir);
        let client = client_for_profile(profile, store.as_ref()).with_context(|| {
            format!(
                "profile {} is not paired; run `bb-tui connect pair <code>`",
                profile.name()
            )
        })?;
        return Ok(Connection {
            info: ServerInfo {
                url: profile.url().to_owned(),
                profile: Some(profile.name().to_owned()),
            },
            source: format!("profile {}", profile.id()),
            client,
        });
    }
    let discovered = discover_server(
        &DiscoveryInput::from_env(args.server_url.clone()),
        process_is_running,
    )?;
    Ok(Connection {
        info: ServerInfo {
            url: discovered.endpoint.to_string(),
            profile: None,
        },
        source: discovered.source.label(),
        client: BbClient::local(discovered.endpoint)?,
    })
}

pub fn save_profile(dir: &Path, profile: ServerProfile, activate: bool) -> Result<ProfileList> {
    let store = ProfileStore::new(dir);
    let mut list = store.load()?;
    let id = profile.id().to_owned();
    list.upsert(profile);
    if activate {
        list.set_active(&id);
    }
    store.save(&list)?;
    Ok(list)
}

pub fn rename_profile(profile: ServerProfile, name: Option<String>) -> ServerProfile {
    let Some(new_name) = name.filter(|name| !name.trim().is_empty()) else {
        return profile;
    };
    match profile {
        ServerProfile::Local { id, url, .. } => ServerProfile::Local {
            id,
            name: new_name,
            url,
        },
        ServerProfile::Connect {
            id,
            server_url,
            handle,
            apex_url,
            ..
        } => ServerProfile::Connect {
            id,
            name: new_name,
            server_url,
            handle,
            apex_url,
        },
    }
}

pub fn format_profiles(list: &ProfileList) -> String {
    if list.profiles.is_empty() {
        return "No saved profiles. Pair one with `bb-tui connect pair <code>`.\n".into();
    }
    let mut out = String::new();
    for profile in &list.profiles {
        let marker = if list.active.as_deref() == Some(profile.id()) {
            '*'
        } else {
            ' '
        };
        let kind = if profile.is_connect() {
            "connect"
        } else {
            "local"
        };
        out.push_str(&format!(
            "{marker} {}  {}  ({kind}, id {})\n",
            profile.name(),
            profile.url(),
            profile.id()
        ));
    }
    out
}

pub async fn run_connect_command(args: &Args, command: ConnectCommand) -> Result<()> {
    let dir = config_dir(args)?;
    match command {
        ConnectCommand::List => {
            print!("{}", format_profiles(&ProfileStore::new(&dir).load()?));
        }
        ConnectCommand::Pair {
            code,
            name,
            device_name,
            no_activate,
        } => {
            let Some(input) = parse_pairing_input(&code) else {
                bail!("{code:?} is not a bb connect machine code or pairing link");
            };
            let credentials = default_credential_store(&dir);
            let profile =
                pair_connect_profile(&input, device_name.as_deref(), credentials.as_ref()).await?;
            let profile = rename_profile(profile, name);
            let summary = format!(
                "Paired {} ({}). Credential stored in {}.\nStart with: bb-tui --profile {}\n",
                profile.name(),
                profile.url(),
                credentials.describe(),
                profile.name()
            );
            save_profile(&dir, profile, !no_activate)?;
            print!("{summary}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connect_profile(handle: &str) -> ServerProfile {
        ServerProfile::Connect {
            id: format!("connect:{handle}"),
            name: handle.to_uppercase(),
            server_url: format!("https://{handle}.example.test"),
            handle: handle.into(),
            apex_url: "https://example.test".into(),
        }
    }

    #[test]
    fn profiles_resolve_by_id_name_handle_or_url() {
        let mut list = ProfileList::default();
        list.upsert(connect_profile("bee"));
        list.upsert(ServerProfile::local("Laptop", "http://127.0.0.1:4100"));
        assert_eq!(
            find_profile(&list, "connect:bee").map(ServerProfile::id),
            Some("connect:bee")
        );
        assert_eq!(
            find_profile(&list, "BEE").map(ServerProfile::id),
            Some("connect:bee")
        );
        assert_eq!(
            find_profile(&list, "bee").map(ServerProfile::id),
            Some("connect:bee")
        );
        assert_eq!(
            find_profile(&list, "http://127.0.0.1:4100").map(ServerProfile::name),
            Some("Laptop")
        );
        assert!(find_profile(&list, "wasp").is_none());
    }

    #[test]
    fn save_profile_upserts_and_activates() {
        let dir = tempfile::tempdir().unwrap();
        save_profile(dir.path(), connect_profile("bee"), false).unwrap();
        let list = save_profile(
            dir.path(),
            rename_profile(connect_profile("bee"), Some("Office".into())),
            true,
        )
        .unwrap();
        assert_eq!(list.profiles.len(), 1);
        assert_eq!(list.active.as_deref(), Some("connect:bee"));
        let listing = format_profiles(&ProfileStore::new(dir.path()).load().unwrap());
        assert_eq!(
            listing,
            "* Office  https://bee.example.test  (connect, id connect:bee)\n"
        );
    }

    #[test]
    fn unpaired_profile_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        save_profile(dir.path(), connect_profile("bee"), true).unwrap();
        let args = Args::parse_from([
            "bb-tui",
            "--profile",
            "bee",
            "--config-dir",
            dir.path().to_str().unwrap(),
        ]);
        let error = resolve_connection(&args).err().expect("unpaired");
        assert!(error.to_string().contains("not paired"), "{error}");
    }

    #[test]
    fn server_url_and_profile_conflict() {
        assert!(
            Args::try_parse_from(["bb-tui", "--server-url", "http://x", "--profile", "p"]).is_err()
        );
        let args = Args::try_parse_from(["bb-tui", "connect", "pair", "ABCD-1234"]).unwrap();
        assert!(matches!(
            args.command,
            Some(TopCommand::Connect(ConnectCommand::Pair { .. }))
        ));
    }
}
