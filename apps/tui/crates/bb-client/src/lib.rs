pub mod api;
pub mod auth;
pub mod connect;
pub mod discovery;
pub mod endpoint;
pub mod error;
pub mod http;
pub mod profiles;
pub mod realtime;
pub mod terminal;
mod ws;

pub use auth::Auth;
pub use connect::{ConnectCredential, ConnectError, ConnectSession};
pub use discovery::{
    DiscoveredServer, DiscoveryInput, ProbeReport, ServerUrlSource, discover_server, probe_server,
};
pub use endpoint::ServerEndpoint;
pub use error::{ClientError, HttpError};
pub use http::{BbClient, ClientConfig};
pub use profiles::{CredentialStore, ProfileList, ProfileStore, ServerProfile};
pub use realtime::{RealtimeClient, RealtimeConfig, RealtimeEvent, Subscription};
pub use terminal::{TerminalConfig, TerminalConnection, TerminalConnectionState, TerminalEvent};
