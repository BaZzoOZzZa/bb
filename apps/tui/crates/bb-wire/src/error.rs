use std::fmt;

use serde::{Deserialize, Serialize};

use crate::common::JsonValue;
use crate::environment::{EnvironmentStatus, HostStatus};
use crate::thread::ThreadStatus;

pub mod codes {
    pub const ENVIRONMENT_NOT_READY: &str = "environment_not_ready";
    pub const THREAD_NOT_WRITABLE: &str = "thread_not_writable";
    pub const THREAD_ENVIRONMENT_UNAVAILABLE: &str = "thread_environment_unavailable";
    pub const HOST_UNAVAILABLE: &str = "host_unavailable";
    pub const PROJECT_UNAVAILABLE: &str = "project_unavailable";
    pub const PARENT_THREAD_INVALID: &str = "parent_thread_invalid";
    pub const DISPATCH_REJECTED: &str = "dispatch_rejected";
    pub const DISPATCH_HOOK_FAILED: &str = "dispatch_hook_failed";
    pub const SERVER_MOVED: &str = "server_moved";
    pub const SERVER_MOVING: &str = "server_moving";
    pub const FORBIDDEN_HOST: &str = "forbidden_host";
    pub const FORBIDDEN_ORIGIN: &str = "forbidden_origin";
    pub const INVALID_REQUEST: &str = "invalid_request";
    pub const NOT_FOUND: &str = "not_found";
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ApiError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retryable: Option<bool>,
}

impl ApiError {
    pub fn is_retryable(&self) -> bool {
        self.retryable.unwrap_or(false)
    }

    pub fn server_moved_details(&self) -> Option<ServerMovedDetails> {
        if self.code != codes::SERVER_MOVED {
            return None;
        }
        serde_json::from_value(self.details.clone()?).ok()
    }

    pub fn lifecycle_details(&self) -> Option<LifecycleErrorDetails> {
        let details = self.details.clone()?;
        let parsed = match self.code.as_str() {
            codes::ENVIRONMENT_NOT_READY => {
                LifecycleErrorDetails::EnvironmentNotReady(serde_json::from_value(details).ok()?)
            }
            codes::THREAD_NOT_WRITABLE => {
                LifecycleErrorDetails::ThreadNotWritable(serde_json::from_value(details).ok()?)
            }
            codes::THREAD_ENVIRONMENT_UNAVAILABLE => {
                LifecycleErrorDetails::ThreadEnvironmentUnavailable(
                    serde_json::from_value(details).ok()?,
                )
            }
            codes::HOST_UNAVAILABLE => {
                LifecycleErrorDetails::HostUnavailable(serde_json::from_value(details).ok()?)
            }
            codes::PROJECT_UNAVAILABLE => {
                LifecycleErrorDetails::ProjectUnavailable(serde_json::from_value(details).ok()?)
            }
            codes::PARENT_THREAD_INVALID => {
                LifecycleErrorDetails::ParentThreadInvalid(serde_json::from_value(details).ok()?)
            }
            codes::DISPATCH_REJECTED => {
                LifecycleErrorDetails::DispatchRejected(serde_json::from_value(details).ok()?)
            }
            codes::DISPATCH_HOOK_FAILED => {
                LifecycleErrorDetails::DispatchHookFailed(serde_json::from_value(details).ok()?)
            }
            _ => return None,
        };
        Some(parsed)
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ApiError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerMovedDetails {
    pub server_url: String,
    #[serde(default)]
    pub to_host_name: String,
    #[serde(default)]
    pub moved_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EnvironmentNotReadyDetails {
    pub environment_status: EnvironmentStatus,
    pub has_path: bool,
}

wire_enum! {
    pub enum ThreadNotWritableReason {
        Archived => "archived",
        Stopping => "stopping",
        Deleted => "deleted",
        NotStarted => "not_started",
        NotActive => "not_active",
        Errored => "errored",
        AlreadyActive => "already_active",
        StillStarting => "still_starting",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ThreadNotWritableDetails {
    pub reason: ThreadNotWritableReason,
    pub archived_at: Option<i64>,
    pub thread_status: ThreadStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ThreadEnvironmentUnavailableDetails {
    pub reason: String,
    pub environment_status: Option<EnvironmentStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HostUnavailableDetails {
    pub reason: String,
    pub host_status: Option<HostStatus>,
    pub suspended_at: Option<i64>,
    pub destroyed_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProjectUnavailableDetails {
    pub reason: String,
    pub deleted_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ParentThreadInvalidDetails {
    pub reason: String,
    pub subject: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DispatchHookDetails {
    pub plugin_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifecycleErrorDetails {
    EnvironmentNotReady(EnvironmentNotReadyDetails),
    ThreadNotWritable(ThreadNotWritableDetails),
    ThreadEnvironmentUnavailable(ThreadEnvironmentUnavailableDetails),
    HostUnavailable(HostUnavailableDetails),
    ProjectUnavailable(ProjectUnavailableDetails),
    ParentThreadInvalid(ParentThreadInvalidDetails),
    DispatchRejected(DispatchHookDetails),
    DispatchHookFailed(DispatchHookDetails),
}
