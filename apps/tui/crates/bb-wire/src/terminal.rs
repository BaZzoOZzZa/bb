use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};

use crate::common::JsonValue;

pub const TERMINAL_COLS_MAX: u16 = 500;
pub const TERMINAL_ROWS_MAX: u16 = 200;
pub const TERMINAL_DATA_MAX_BYTES: usize = 64 * 1024;

wire_enum! {
    pub enum TerminalSessionStatus {
        Starting => "starting",
        Running => "running",
        Disconnected => "disconnected",
        Exited => "exited",
    }
}

impl TerminalSessionStatus {
    pub fn is_active(self) -> bool {
        matches!(self, Self::Starting | Self::Running)
    }
}

wire_enum! {
    pub enum TerminalCloseReason {
        User => "user",
        ProcessExit => "process-exit",
        DaemonDisconnect => "daemon-disconnect",
        EnvironmentDestroyed => "environment-destroyed",
        ThreadArchived => "thread-archived",
        ThreadDeleted => "thread-deleted",
        OpenTimeout => "open-timeout",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TerminalSession {
    pub id: String,
    pub thread_id: Option<String>,
    pub environment_id: Option<String>,
    pub host_id: String,
    pub title: String,
    pub initial_cwd: String,
    pub cols: u16,
    pub rows: u16,
    pub status: TerminalSessionStatus,
    pub exit_code: Option<i64>,
    pub close_reason: Option<TerminalCloseReason>,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_user_input_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TerminalListResponse {
    pub sessions: Vec<TerminalSession>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalListQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum TerminalCreateTarget {
    Thread {
        thread_id: String,
    },
    Environment {
        environment_id: String,
    },
    HostPath {
        host_id: String,
        cwd: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "mode",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum TerminalStart {
    Shell,
    Command { command: String },
    Argv { argv: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTerminalRequest {
    pub cols: u16,
    pub rows: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<TerminalStart>,
    pub target: TerminalCreateTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

wire_enum! {
    pub enum TerminalCloseMode {
        Force => "force",
        IfClean => "if-clean",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloseTerminalRequest {
    pub mode: TerminalCloseMode,
    pub reason: TerminalCloseReason,
}

impl CloseTerminalRequest {
    pub fn by_user(mode: TerminalCloseMode) -> Self {
        Self {
            mode,
            reason: TerminalCloseReason::User,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RestartTerminalRequest {}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateTerminalRequest {
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalInputRequest {
    pub data_base64: String,
}

impl TerminalInputRequest {
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self {
            data_base64: STANDARD.encode(bytes),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalResizeRequest {
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalOutputQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since_seq: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tail_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_chunks: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TerminalOutputChunk {
    pub seq: u64,
    pub data_base64: String,
}

impl TerminalOutputChunk {
    pub fn decode(&self) -> Result<Vec<u8>, base64::DecodeError> {
        STANDARD.decode(&self.data_base64)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TerminalOutputResponse {
    pub chunks: Vec<TerminalOutputChunk>,
    pub next_seq: u64,
    pub truncated: bool,
    pub status: TerminalSessionStatus,
    pub exit_code: Option<i64>,
    pub close_reason: Option<TerminalCloseReason>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalWebSocketQuery {
    pub since_seq: u64,
    #[serde(with = "zero_one")]
    pub output_acks: bool,
}

mod zero_one {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &bool, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(if *value { "1" } else { "0" })
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
        Ok(String::deserialize(deserializer)? == "1")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum TerminalClientMessage {
    Input { data_base64: String },
    Resize { cols: u16, rows: u16 },
    Ping,
    Ack { next_seq: u64 },
    Visibility { visible: bool },
}

impl TerminalClientMessage {
    pub fn input(bytes: &[u8]) -> Self {
        Self::Input {
            data_base64: STANDARD.encode(bytes),
        }
    }

    pub fn input_chunks(bytes: &[u8]) -> Vec<Self> {
        bytes
            .chunks(TERMINAL_DATA_MAX_BYTES)
            .map(Self::input)
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum TerminalServerMessage {
    Attached {
        #[serde(default)]
        session: TerminalSession,
        #[serde(default)]
        replay_start_seq: u64,
        #[serde(default)]
        next_seq: u64,
    },
    Output {
        chunk: TerminalOutputChunk,
    },
    SessionUpdated {
        #[serde(default)]
        session: TerminalSession,
    },
    Exited {
        #[serde(default)]
        session: TerminalSession,
    },
    Error {
        #[serde(default)]
        code: String,
        #[serde(default)]
        message: String,
    },
    Pong,
    #[serde(untagged)]
    Unknown(JsonValue),
}

default_unknown!(TerminalServerMessage);
