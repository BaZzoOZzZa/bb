use serde::{Deserialize, Serialize};

use crate::environment::{GitBranchRefClassification, GitCheckoutRef};
use crate::prompt::{PathEntryKind, ProjectExecutionDefaults};
use crate::thread::ThreadListEntry;

pub const PERSONAL_PROJECT_ID: &str = "proj_personal";

wire_enum! {
    pub enum ProjectKind {
        Standard => "standard",
        Personal => "personal",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Project {
    pub id: String,
    pub kind: ProjectKind,
    pub name: String,
    pub git_remote_url: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProjectSource {
    pub id: String,
    pub project_id: String,
    #[serde(rename = "type")]
    pub source_type: String,
    pub host_id: String,
    pub path: String,
    pub is_default: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProjectResponse {
    #[serde(flatten)]
    pub project: Project,
    pub sources: Vec<ProjectSource>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProjectWithThreadsResponse {
    #[serde(flatten)]
    pub project: Project,
    pub sources: Vec<ProjectSource>,
    pub threads: Vec<ThreadListEntry>,
    pub default_execution_options: Option<ProjectExecutionDefaults>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ThreadSection {
    pub id: String,
    pub name: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SidebarBootstrapResponse {
    pub sections: Vec<ThreadSection>,
    pub projects: Vec<ProjectWithThreadsResponse>,
    pub personal_project: ProjectWithThreadsResponse,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectListQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_personal: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalPathProjectSourceRequest {
    pub host_id: String,
    #[serde(rename = "type")]
    pub source_type: String,
    pub path: String,
}

impl LocalPathProjectSourceRequest {
    pub fn new(host_id: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            host_id: host_id.into(),
            source_type: "local_path".into(),
            path: path.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateProjectRequest {
    pub name: String,
    pub source: LocalPathProjectSourceRequest,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProjectRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReorderProjectRequest {
    pub previous_project_id: Option<String>,
    pub next_project_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateThreadSectionRequest {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateThreadSectionRequest {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteThreadSectionRequest {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ThreadSectionMutationResponse {
    pub id: String,
    pub name: String,
    pub updated_thread_count: u32,
}

wire_enum! {
    pub enum ProviderCommandSource {
        Skill => "skill",
        Command => "command",
    }
}

wire_enum! {
    pub enum ProviderCommandOrigin {
        Builtin => "builtin",
        Project => "project",
        User => "user",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProviderCommand {
    pub name: String,
    pub source: ProviderCommandSource,
    pub origin: ProviderCommandOrigin,
    pub description: Option<String>,
    pub argument_hint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CommandListResponse {
    pub commands: Vec<ProviderCommand>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectCommandsQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment_id: Option<String>,
    pub provider: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFilesQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspacePathsQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    pub include_files: bool,
    pub include_directories: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkspaceFile {
    pub path: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkspaceFileListResponse {
    pub files: Vec<WorkspaceFile>,
    pub truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage_root_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkspacePathEntry {
    pub kind: PathEntryKind,
    pub path: String,
    pub name: String,
    pub score: f64,
    pub positions: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkspacePathListResponse {
    pub paths: Vec<WorkspacePathEntry>,
    pub truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage_root_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectBranchesQuery {
    pub host_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_branch: Option<String>,
}

wire_enum! {
    pub enum DefaultBranchRelation {
        Equal => "equal",
        LocalBehind => "local-behind",
        LocalAhead => "local-ahead",
        Diverged => "diverged",
    }
}

wire_enum! {
    pub enum WorkspaceGitOperationKind {
        None => "none",
        Merge => "merge",
        Rebase => "rebase",
        CherryPick => "cherry-pick",
        Revert => "revert",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkspaceGitOperation {
    pub kind: WorkspaceGitOperationKind,
    pub has_conflicts: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProjectBranchesResponse {
    pub checkout: GitCheckoutRef,
    pub default_branch: Option<String>,
    pub is_worktree: bool,
    pub default_branch_relation: Option<DefaultBranchRelation>,
    pub has_uncommitted_changes: bool,
    pub operation: WorkspaceGitOperation,
    pub origin_default_branch: Option<String>,
    pub branches: Vec<String>,
    pub branches_truncated: bool,
    pub remote_branches: Vec<String>,
    pub remote_branches_truncated: bool,
    pub selected_branch: Option<GitBranchRefClassification>,
    pub default_worktree_base_branch: Option<String>,
}

wire_enum! {
    pub enum UploadedAttachmentType {
        LocalImage => "localImage",
        LocalFile => "localFile",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UploadedPromptAttachment {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_project_id: Option<String>,
    #[serde(rename = "type")]
    pub attachment_type: UploadedAttachmentType,
    pub path: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptHistoryQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}
