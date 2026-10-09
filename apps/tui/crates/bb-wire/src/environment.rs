use serde::{Deserialize, Serialize};

use crate::common::JsonValue;
use crate::prompt::PermissionMode;

wire_enum! {
    pub enum EnvironmentStatus {
        Creating => "creating",
        Provisioning => "provisioning",
        Ready => "ready",
        Error => "error",
        Destroyed => "destroyed",
    }
}

wire_enum! {
    pub enum EnvironmentHostLifecycle {
        Active => "active",
        Removing => "removing",
        CleanupFailed => "cleanup-failed",
        Removed => "removed",
    }
}

wire_enum! {
    pub enum WorkspaceProvisionType {
        Unmanaged => "unmanaged",
        ManagedWorktree => "managed-worktree",
        Personal => "personal",
    }
}

wire_enum! {
    pub enum EnvironmentWorkspaceDisplayKind {
        ManagedWorktree => "managed-worktree",
        UnmanagedWorktree => "unmanaged-worktree",
        Other => "other",
    }
}

wire_enum! {
    pub enum EnvironmentLifecyclePhase {
        Active => "active",
        Retiring => "retiring",
        Teardown => "teardown",
        Destroyed => "destroyed",
    }
}

wire_enum! {
    pub enum TeardownStatus {
        Running => "running",
        Failed => "failed",
        Removed => "removed",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Teardown {
    pub status: TeardownStatus,
    pub attempt: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EnvironmentLifecycle {
    pub phase: EnvironmentLifecyclePhase,
    pub retire_at: Option<i64>,
    pub teardown: Option<Teardown>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Environment {
    pub id: String,
    pub name: Option<String>,
    pub project_id: String,
    pub host_id: String,
    pub path: Option<String>,
    pub is_git_repo: bool,
    pub is_worktree: bool,
    pub branch_name: Option<String>,
    pub base_branch: Option<String>,
    pub default_branch: Option<String>,
    pub merge_base_branch: Option<String>,
    pub status: EnvironmentStatus,
    pub environment_provider_id: Option<String>,
    pub lifecycle: EnvironmentLifecycle,
    pub host_lifecycle: EnvironmentHostLifecycle,
    pub environment_provider_selection: Option<JsonValue>,
    pub environment_provider_instance_key: Option<String>,
    pub managed: bool,
    pub workspace_provision_type: Option<WorkspaceProvisionType>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Environment {
    pub fn resolved_merge_base_branch(&self) -> Option<&str> {
        self.merge_base_branch
            .as_deref()
            .or(self.base_branch.as_deref())
            .or(self.default_branch.as_deref())
    }
}

wire_enum! {
    pub enum HostStatus {
        Connected => "connected",
        Disconnected => "disconnected",
    }
}

wire_enum! {
    pub enum HostType {
        Persistent => "persistent",
        Ephemeral => "ephemeral",
    }
}

wire_enum! {
    pub enum MachinePhase {
        Creating => "creating",
        Active => "active",
        Suspending => "suspending",
        Suspended => "suspended",
        Resuming => "resuming",
        Removing => "removing",
        Destroyed => "destroyed",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MachineLifecycle {
    pub phase: MachinePhase,
    pub suspended_at: Option<i64>,
    pub message: Option<String>,
    pub pending_log: String,
    pub teardown: Option<Teardown>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Host {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub host_type: HostType,
    pub status: HostStatus,
    pub machine_provider_id: Option<String>,
    pub lifecycle: MachineLifecycle,
    pub max_permission_mode: PermissionMode,
    pub last_seen_at: Option<i64>,
    pub last_rejected_protocol_version: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connect_machine_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_storage_root_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum GitBranchSelection {
    Named { name: String },
    Default,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum UnmanagedBranchSpec {
    Existing { name: String },
    New { base_branch: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum WorkspaceArgs {
    Unmanaged {
        path: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        branch: Option<UnmanagedBranchSpec>,
    },
    ManagedWorktree {
        base_branch: GitBranchSelection,
    },
    Personal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum MachineSelection {
    Existing {
        host_id: String,
    },
    New {
        machine_provider_id: String,
        #[serde(default)]
        inputs: Option<JsonValue>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum CreateThreadEnvironmentArgs {
    Reuse {
        environment_id: String,
    },
    Host {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        host_id: Option<String>,
        workspace: WorkspaceArgs,
    },
    ProjectDefault,
    Provider {
        environment_provider_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        machine: Option<MachineSelection>,
        #[serde(default)]
        inputs: Option<JsonValue>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListEnvironmentsQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment_provider_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<EnvironmentStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateEnvironmentRequest {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::common::double_option"
    )]
    pub merge_base_branch: Option<Option<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::common::double_option"
    )]
    pub name: Option<Option<String>>,
}

wire_enum! {
    pub enum WorkspaceState {
        Clean => "clean",
        Untracked => "untracked",
        DirtyUncommitted => "dirty_uncommitted",
        CommittedUnmerged => "committed_unmerged",
        DirtyAndCommittedUnmerged => "dirty_and_committed_unmerged",
    }
}

wire_enum! {
    pub enum WorkspaceFileStatusKind {
        Modified => "M",
        Added => "A",
        Deleted => "D",
        Renamed => "R",
        Copied => "C",
        Unmerged => "U",
        Untracked => "??",
        UntrackedShort => "?",
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkspaceFileStatus {
    pub path: String,
    pub status: WorkspaceFileStatusKind,
    pub insertions: Option<i64>,
    pub deletions: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkspaceWorkingTree {
    pub insertions: i64,
    pub deletions: i64,
    pub line_stats_complete: bool,
    pub files: Vec<WorkspaceFileStatus>,
    pub has_uncommitted_changes: bool,
    pub state: WorkspaceState,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkspaceCommitSummary {
    pub sha: String,
    pub short_sha: String,
    pub subject: String,
    pub author_name: String,
    pub authored_at: i64,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkspaceMergeBase {
    pub insertions: i64,
    pub deletions: i64,
    pub line_stats_complete: bool,
    pub files: Vec<WorkspaceFileStatus>,
    pub merge_base_branch: String,
    pub base_ref: Option<String>,
    pub ahead_count: i64,
    pub behind_count: i64,
    pub has_committed_unmerged_changes: bool,
    pub commits: Vec<WorkspaceCommitSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkspaceBranch {
    pub current_branch: Option<String>,
    pub default_branch: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum GitCheckoutRef {
    Branch {
        branch_name: String,
        #[serde(default)]
        head_sha: Option<String>,
    },
    Detached {
        #[serde(default)]
        head_sha: Option<String>,
    },
    Unborn {
        #[serde(default)]
        branch_name: Option<String>,
    },
    #[serde(rename = "unknown")]
    Unresolved {
        #[serde(default)]
        reason: String,
    },
    #[serde(untagged)]
    Unknown(JsonValue),
}

default_unknown!(GitCheckoutRef);

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkspaceStatus {
    pub working_tree: WorkspaceWorkingTree,
    pub checkout: GitCheckoutRef,
    pub branch: WorkspaceBranch,
    pub merge_base: Option<WorkspaceMergeBase>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkspaceResolutionFailure {
    pub code: String,
    pub workspace_path: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "outcome",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum EnvironmentStatusResponse {
    Available {
        workspace: Box<WorkspaceStatus>,
    },
    NotApplicable {
        #[serde(default)]
        reason: String,
        #[serde(default)]
        message: String,
    },
    Unavailable {
        failure: WorkspaceResolutionFailure,
    },
    #[serde(untagged)]
    Unknown(JsonValue),
}

default_unknown!(EnvironmentStatusResponse);

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ThreadGitDiff {
    pub diff: String,
    pub truncated: bool,
    pub shortstat: String,
    pub files: String,
    pub merge_base_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "outcome",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum EnvironmentDiffResponse {
    Available {
        diff: ThreadGitDiff,
    },
    NotApplicable {
        #[serde(default)]
        reason: String,
        #[serde(default)]
        message: String,
    },
    Unavailable {
        failure: WorkspaceResolutionFailure,
    },
    #[serde(untagged)]
    Unknown(JsonValue),
}

default_unknown!(EnvironmentDiffResponse);

wire_enum! {
    pub enum GitDiffFileChangeKind {
        Added => "added",
        Modified => "modified",
        Deleted => "deleted",
        Renamed => "renamed",
        Copied => "copied",
        TypeChanged => "type_changed",
    }
}

wire_enum! {
    pub enum DiffFileOrigin {
        Tracked => "tracked",
        Untracked => "untracked",
    }
}

wire_enum! {
    pub enum DiffFileLoadMode {
        Auto => "auto",
        OnDemand => "on_demand",
        TooLarge => "too_large",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DiffFileEntry {
    pub path: String,
    pub previous_path: Option<String>,
    pub change_kind: GitDiffFileChangeKind,
    pub additions: u64,
    pub deletions: u64,
    pub binary: bool,
    pub origin: DiffFileOrigin,
    pub load_mode: DiffFileLoadMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DiffPatchEntry {
    pub path: String,
    pub patch: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "outcome",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum EnvironmentDiffFilesResponse {
    Available {
        #[serde(default)]
        files: Vec<DiffFileEntry>,
        #[serde(default)]
        truncated: bool,
        #[serde(default)]
        shortstat: String,
        #[serde(default)]
        merge_base_ref: Option<String>,
        #[serde(default)]
        initial_patches: Vec<DiffPatchEntry>,
    },
    NotApplicable {
        #[serde(default)]
        reason: String,
        #[serde(default)]
        message: String,
    },
    Unavailable {
        failure: WorkspaceResolutionFailure,
    },
    #[serde(untagged)]
    Unknown(JsonValue),
}

default_unknown!(EnvironmentDiffFilesResponse);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "outcome",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum EnvironmentDiffPatchResponse {
    Available {
        #[serde(default)]
        patches: Vec<DiffPatchEntry>,
    },
    NotApplicable {
        #[serde(default)]
        reason: String,
        #[serde(default)]
        message: String,
    },
    Unavailable {
        failure: WorkspaceResolutionFailure,
    },
    #[serde(untagged)]
    Unknown(JsonValue),
}

default_unknown!(EnvironmentDiffPatchResponse);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum WorkspaceDiffTarget {
    Uncommitted,
    BranchCommitted { merge_base_branch: String },
    All { merge_base_branch: String },
    Commit { sha: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "target",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum EnvironmentDiffQuery {
    Uncommitted,
    BranchCommitted { merge_base_branch: String },
    All { merge_base_branch: String },
    Commit { sha: String },
}

wire_enum! {
    pub enum DiffFileSide {
        Old => "old",
        New => "new",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "target",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum EnvironmentDiffFileQuery {
    Uncommitted {
        path: String,
        side: DiffFileSide,
    },
    BranchCommitted {
        merge_base_ref: String,
        path: String,
        side: DiffFileSide,
    },
    All {
        merge_base_ref: String,
        path: String,
        side: DiffFileSide,
    },
    Commit {
        sha: String,
        path: String,
        side: DiffFileSide,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentDiffPatchRequest {
    pub target: WorkspaceDiffTarget,
    pub paths: Vec<String>,
}

wire_enum! {
    pub enum ContentEncoding {
        Base64 => "base64",
        Utf8 => "utf8",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EnvironmentDiffFileResponse {
    pub path: String,
    pub content: String,
    pub content_encoding: ContentEncoding,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchListQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_branch: Option<String>,
}

wire_enum! {
    pub enum GitBranchRefKind {
        Local => "local",
        Remote => "remote",
        Missing => "missing",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GitBranchRefClassification {
    pub name: String,
    pub kind: GitBranchRefKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GitBranchOptions {
    pub branches: Vec<String>,
    pub branches_truncated: bool,
    pub remote_branches: Vec<String>,
    pub remote_branches_truncated: bool,
    pub selected_branch: Option<GitBranchRefClassification>,
}

wire_enum! {
    pub enum PullRequestState {
        Draft => "draft",
        Open => "open",
        Merged => "merged",
        Closed => "closed",
    }
}

wire_enum! {
    pub enum PullRequestChecksState {
        Passing => "passing",
        Failing => "failing",
        Pending => "pending",
        NoChecks => "no_checks",
    }
}

wire_enum! {
    pub enum PullRequestReviewState {
        Approved => "approved",
        ChangesRequested => "changes_requested",
        ReviewRequired => "review_required",
        ReviewRequested => "review_requested",
        None => "none",
    }
}

wire_enum! {
    pub enum PullRequestMergeabilityState {
        Mergeable => "mergeable",
        Conflicts => "conflicts",
        Blocked => "blocked",
        Draft => "draft",
    }
}

wire_enum! {
    pub enum PullRequestAttentionState {
        ChecksFailed => "checks_failed",
        ChecksPending => "checks_pending",
        ChangesRequested => "changes_requested",
        ReviewRequested => "review_requested",
        Conflicts => "conflicts",
        Blocked => "blocked",
        Draft => "draft",
        Queued => "queued",
        ReadyToMerge => "ready_to_merge",
        Merged => "merged",
        Closed => "closed",
        None => "none",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PullRequestChecks {
    pub state: PullRequestChecksState,
    pub total_count: u32,
    pub passed_count: u32,
    pub failed_count: u32,
    pub pending_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PullRequestReview {
    pub state: PullRequestReviewState,
    pub review_request_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PullRequestMergeability {
    pub state: PullRequestMergeabilityState,
    pub merge_state_status: Option<String>,
    pub mergeable: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ThreadPullRequest {
    pub number: u64,
    pub title: String,
    pub state: PullRequestState,
    pub url: String,
    pub base_ref_name: String,
    pub head_ref_name: String,
    pub updated_at: String,
    pub auto_merge: bool,
    pub in_merge_queue: Option<bool>,
    pub checks: PullRequestChecks,
    pub review: PullRequestReview,
    pub mergeability: PullRequestMergeability,
    pub attention: PullRequestAttentionState,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "outcome",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum EnvironmentPullRequestResponse {
    Available {
        pull_request: ThreadPullRequest,
    },
    Absent,
    Unavailable {
        #[serde(default)]
        message: String,
    },
    #[serde(untagged)]
    Unknown(JsonValue),
}

default_unknown!(EnvironmentPullRequestResponse);

wire_enum! {
    pub enum PullRequestMergeMethod {
        Merge => "merge",
        Squash => "squash",
        Rebase => "rebase",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestMergeOptions {
    pub method: PullRequestMergeMethod,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "action",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum EnvironmentActionRequest {
    Commit,
    PullRequestReady,
    PullRequestMerge { options: PullRequestMergeOptions },
    PullRequestDraft,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "action",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum EnvironmentActionResponse {
    Commit {
        #[serde(default)]
        message: String,
        #[serde(default)]
        commit_sha: String,
        #[serde(default)]
        commit_subject: String,
    },
    PullRequestReady {
        #[serde(default)]
        message: String,
    },
    PullRequestMerge {
        #[serde(default)]
        method: PullRequestMergeMethod,
        #[serde(default)]
        message: String,
    },
    PullRequestDraft {
        #[serde(default)]
        message: String,
    },
    #[serde(untagged)]
    Unknown(JsonValue),
}

default_unknown!(EnvironmentActionResponse);

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EnvironmentArchiveThreadsResponse {
    pub ok: bool,
    pub archived_thread_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentStatusQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merge_base_branch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateHostRequest {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateHostPermissionCeilingRequest {
    pub max_permission_mode: PermissionMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HostProviderCliStatus {
    pub display_name: String,
    pub executable_name: Option<String>,
    pub executable_path: Option<String>,
    pub installed: bool,
    pub install_source: Option<String>,
    pub current_version: Option<String>,
    pub latest_version: Option<String>,
    pub minimum_supported_version: Option<String>,
    pub needs_update: bool,
    pub version_unsupported: bool,
}

pub type HostProviderCliStatusResponse = std::collections::BTreeMap<String, HostProviderCliStatus>;
