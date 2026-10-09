use std::cell::Cell;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use bb_wire::environment::{
    DiffFileSide, DiffPatchEntry, Environment, EnvironmentActionRequest, EnvironmentActionResponse,
    EnvironmentDiffFileQuery, EnvironmentDiffFileResponse, EnvironmentDiffFilesResponse,
    EnvironmentDiffPatchResponse, EnvironmentPullRequestResponse, EnvironmentStatusResponse,
    GitDiffFileChangeKind, PullRequestMergeMethod, PullRequestMergeOptions, PullRequestState,
    ThreadPullRequest, WorkspaceDiffTarget, WorkspaceFileStatusKind, WorkspaceStatus,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Color, Style};
use ratatui::text::Span;

use crate::api::{Mutation, MutationReply};
use crate::app::{AppState, Effect, Focus, ToastLevel};
use crate::execution::Load;
use crate::git_diff::{
    DiffDocument, DiffListing, DiffRow, DiffScope, diff_file_query, scope_options,
    should_reset_scope,
};

pub const PREVIEW_MAX_BYTES: usize = 512 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PanelTab {
    #[default]
    Info,
    Diff,
    File,
    Terminal,
}

impl PanelTab {
    pub const ALL: [PanelTab; 4] = [
        PanelTab::Info,
        PanelTab::Diff,
        PanelTab::File,
        PanelTab::Terminal,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Info => "Info",
            Self::Diff => "Diff",
            Self::File => "File",
            Self::Terminal => "Terminal",
        }
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|tab| *tab == self).unwrap_or(0)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum StatusView {
    Available(Box<WorkspaceStatus>),
    NotApplicable(String),
    Unavailable(String),
}

impl StatusView {
    pub fn workspace(&self) -> Option<&WorkspaceStatus> {
        match self {
            Self::Available(status) => Some(status),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FilePreview {
    pub path: String,
    pub label: String,
    pub absolute: Option<String>,
    pub line: Option<u32>,
    pub body: Load<Vec<Vec<Span<'static>>>>,
    pub note: Option<String>,
    pub scroll: usize,
    pub column: usize,
    pub viewport: Cell<usize>,
    pub positioned: bool,
}

impl FilePreview {
    pub fn line_count(&self) -> usize {
        self.body.loaded().map_or(1, Vec::len)
    }

    fn scroll_by(&mut self, delta: isize) {
        let max = self.line_count().saturating_sub(self.viewport.get().max(1));
        self.scroll = self.scroll.saturating_add_signed(delta).min(max);
    }

    pub fn position(&mut self) {
        if self.positioned || self.body.loaded().is_none() {
            return;
        }
        self.positioned = true;
        if let Some(line) = self.line {
            let half = self.viewport.get() / 2;
            self.scroll = (line as usize).saturating_sub(1).saturating_sub(half);
            self.scroll_by(0);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitAction {
    Commit,
    MarkReady,
    ConvertToDraft,
    Merge,
}

impl GitAction {
    pub fn title(&self) -> &'static str {
        match self {
            Self::Commit => "Commit changes",
            Self::MarkReady => "Mark pull request ready for review",
            Self::ConvertToDraft => "Convert pull request to draft",
            Self::Merge => "Merge pull request",
        }
    }
}

pub const MERGE_METHODS: [PullRequestMergeMethod; 3] = [
    PullRequestMergeMethod::Squash,
    PullRequestMergeMethod::Merge,
    PullRequestMergeMethod::Rebase,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitConfirm {
    pub action: GitAction,
    pub thread_id: String,
    pub environment_id: String,
    pub lines: Vec<String>,
    pub method: usize,
}

impl GitConfirm {
    pub fn request(&self) -> EnvironmentActionRequest {
        match self.action {
            GitAction::Commit => EnvironmentActionRequest::Commit,
            GitAction::MarkReady => EnvironmentActionRequest::PullRequestReady,
            GitAction::ConvertToDraft => EnvironmentActionRequest::PullRequestDraft,
            GitAction::Merge => EnvironmentActionRequest::PullRequestMerge {
                options: PullRequestMergeOptions {
                    method: MERGE_METHODS[self.method.min(MERGE_METHODS.len() - 1)],
                },
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PanelRequest {
    Status {
        environment_id: String,
        merge_base_branch: Option<String>,
    },
    PullRequest {
        environment_id: String,
    },
    DiffFiles {
        environment_id: String,
        target: WorkspaceDiffTarget,
    },
    DiffPatches {
        environment_id: String,
        target: WorkspaceDiffTarget,
        paths: Vec<String>,
    },
    File {
        environment_id: String,
        path: String,
        query: EnvironmentDiffFileQuery,
    },
}

impl PanelRequest {
    pub fn slot(&self) -> Option<&'static str> {
        match self {
            Self::Status { .. } => Some("panel-status"),
            Self::PullRequest { .. } => Some("panel-pull-request"),
            Self::DiffFiles { .. } => Some("panel-diff-files"),
            Self::File { .. } => Some("panel-file"),
            Self::DiffPatches { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PanelLoaded {
    Status {
        environment_id: String,
        merge_base_branch: Option<String>,
        result: Result<EnvironmentStatusResponse, String>,
    },
    PullRequest {
        environment_id: String,
        result: Result<EnvironmentPullRequestResponse, String>,
    },
    DiffFiles {
        environment_id: String,
        target: WorkspaceDiffTarget,
        result: Result<EnvironmentDiffFilesResponse, String>,
    },
    DiffPatches {
        environment_id: String,
        target: WorkspaceDiffTarget,
        paths: Vec<String>,
        result: Result<EnvironmentDiffPatchResponse, String>,
    },
    File {
        environment_id: String,
        path: String,
        result: Result<EnvironmentDiffFileResponse, String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InfoTarget {
    Commit(String),
    File { path: String, scope: DiffScope },
    Copy(String),
    Url(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct InfoEntry {
    pub label: Option<String>,
    pub spans: Vec<Span<'static>>,
    pub target: Option<InfoTarget>,
    pub heading: bool,
}

impl InfoEntry {
    fn field(label: &str, spans: Vec<Span<'static>>) -> Self {
        Self {
            label: Some(label.to_owned()),
            spans,
            target: None,
            heading: false,
        }
    }

    fn heading(text: String) -> Self {
        Self {
            label: None,
            spans: vec![Span::styled(text, HEADING)],
            target: None,
            heading: true,
        }
    }

    fn item(spans: Vec<Span<'static>>, target: InfoTarget) -> Self {
        Self {
            label: None,
            spans,
            target: Some(target),
            heading: false,
        }
    }

    fn with_target(mut self, target: InfoTarget) -> Self {
        self.target = Some(target);
        self
    }

    fn blank() -> Self {
        Self {
            label: None,
            spans: Vec::new(),
            target: None,
            heading: false,
        }
    }
}

const HEADING: Style = Style::new().fg(Color::Cyan);
const MUTED: Style = Style::new().fg(Color::DarkGray);
const ADDED: Style = Style::new().fg(Color::Green);
const REMOVED: Style = Style::new().fg(Color::Red);
const WARNING: Style = Style::new().fg(Color::Yellow);

#[derive(Debug, Default)]
pub struct RightPanel {
    pub tab: PanelTab,
    pub maximized: bool,
    pub thread_id: Option<String>,
    pub environment_id: Option<String>,
    pub seen_generation: u64,
    pub status: Load<StatusView>,
    pub status_base: Option<String>,
    pub status_fetching: bool,
    pub pull_request: Load<Option<ThreadPullRequest>>,
    pub info_cursor: usize,
    pub info_scroll: usize,
    pub info_viewport: Cell<usize>,
    pub scope: DiffScope,
    pub diff: Load<DiffDocument>,
    pub diff_target: Option<WorkspaceDiffTarget>,
    pub diff_stale: bool,
    pub reveal: Option<String>,
    pub preview: Option<FilePreview>,
    pub preview_return: Option<PanelTab>,
    pub confirm: Option<GitConfirm>,
}

impl RightPanel {
    fn reset(&mut self, thread_id: Option<String>, environment_id: Option<String>) {
        let tab = self.tab;
        let maximized = self.maximized;
        *self = Self {
            tab,
            maximized,
            thread_id,
            environment_id,
            ..Self::default()
        };
    }

    pub fn workspace(&self) -> Option<&WorkspaceStatus> {
        self.status.loaded().and_then(StatusView::workspace)
    }

    pub fn document(&self) -> Option<&DiffDocument> {
        self.diff.loaded()
    }

    fn document_mut(&mut self) -> Option<&mut DiffDocument> {
        match &mut self.diff {
            Load::Loaded(document) => Some(document),
            _ => None,
        }
    }
}

fn change_letter(kind: WorkspaceFileStatusKind) -> &'static str {
    match kind {
        WorkspaceFileStatusKind::Modified => "M",
        WorkspaceFileStatusKind::Added => "A",
        WorkspaceFileStatusKind::Deleted => "D",
        WorkspaceFileStatusKind::Renamed => "R",
        WorkspaceFileStatusKind::Copied => "C",
        WorkspaceFileStatusKind::Unmerged => "U",
        WorkspaceFileStatusKind::Untracked => "A",
        WorkspaceFileStatusKind::UntrackedShort => "?",
        WorkspaceFileStatusKind::Unknown => " ",
    }
}

pub fn change_style(letter: &str) -> Style {
    match letter {
        "A" | "?" => ADDED,
        "D" => REMOVED,
        "U" => WARNING,
        "R" | "C" | "T" => Style::new().fg(Color::Magenta),
        _ => Style::new().fg(Color::Yellow),
    }
}

pub fn format_branch_comparison(ahead: i64, behind: i64, base: &str) -> String {
    match (ahead > 0, behind > 0) {
        (true, true) => format!("{ahead} ahead, {behind} behind {base}"),
        (true, false) => format!("{ahead} ahead of {base}"),
        (false, true) => format!("{behind} behind {base}"),
        (false, false) => format!("Even with {base}"),
    }
}

pub fn checkout_label(status: &WorkspaceStatus) -> Option<(String, String)> {
    use bb_wire::environment::GitCheckoutRef;
    match &status.checkout {
        GitCheckoutRef::Branch { branch_name, .. } => Some(("Branch".into(), branch_name.clone())),
        GitCheckoutRef::Detached { head_sha } => Some((
            "Detached".into(),
            head_sha
                .as_deref()
                .map(|sha| sha.chars().take(9).collect())
                .unwrap_or_else(|| "HEAD".into()),
        )),
        GitCheckoutRef::Unborn { branch_name } => Some((
            "Branch".into(),
            format!(
                "{} (no commits)",
                branch_name.as_deref().unwrap_or("unborn")
            ),
        )),
        GitCheckoutRef::Unresolved { .. } | GitCheckoutRef::Unknown(_) => status
            .branch
            .current_branch
            .clone()
            .map(|branch| ("Branch".into(), branch)),
    }
}

pub fn pull_request_summary(pull_request: &ThreadPullRequest) -> (String, Style) {
    use bb_wire::environment::{PullRequestAttentionState as A, PullRequestChecksState as C};
    let state = match pull_request.state {
        PullRequestState::Draft => ("Draft", MUTED),
        PullRequestState::Open => ("Open", ADDED),
        PullRequestState::Merged => ("Merged", Style::new().fg(Color::Magenta)),
        PullRequestState::Closed => ("Closed", REMOVED),
        PullRequestState::Unknown => ("Unknown", MUTED),
    };
    let next = match pull_request.attention {
        A::ChecksFailed => Some("checks failed"),
        A::ChecksPending => Some("checks running"),
        A::ChangesRequested => Some("changes requested"),
        A::ReviewRequested => Some("review requested"),
        A::Conflicts => Some("conflicts"),
        A::Blocked => Some("blocked"),
        A::Queued => Some("in merge queue"),
        A::ReadyToMerge => Some("ready to merge"),
        _ => match pull_request.checks.state {
            C::Failing => Some("checks failed"),
            C::Pending => Some("checks running"),
            _ => None,
        },
    };
    let mut text = format!("#{} {}", pull_request.number, state.0);
    if let Some(next) = next {
        text.push_str(" · ");
        text.push_str(next);
    }
    if pull_request.auto_merge {
        text.push_str(" · auto-merge");
    }
    (text, state.1)
}

fn format_time(timestamp: i64, now: i64) -> String {
    let absolute = chrono::DateTime::from_timestamp_millis(timestamp)
        .map(|date| {
            date.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default();
    format!(
        "{absolute} ({})",
        crate::time::compact_relative_time(timestamp, now)
    )
}

fn compact_tokens(value: f64) -> String {
    if value >= 1_000_000.0 {
        format!("{:.1}M", value / 1_000_000.0)
    } else if value >= 1_000.0 {
        format!("{:.0}k", value / 1_000.0)
    } else {
        format!("{value:.0}")
    }
}

fn environment_label(environment: &Environment) -> String {
    use bb_wire::environment::WorkspaceProvisionType as P;
    let kind = match environment.workspace_provision_type {
        Some(P::ManagedWorktree) => "Git worktree",
        Some(P::Personal) => "Personal workspace",
        Some(P::Unmanaged) if environment.is_worktree => "Worktree",
        Some(P::Unmanaged) => "Project checkout",
        _ => "Environment",
    };
    match environment.name.as_deref().filter(|name| !name.is_empty()) {
        Some(name) => format!("{name} · {kind}"),
        None => kind.to_owned(),
    }
}

fn home_relative(path: &str) -> String {
    match dirs::home_dir() {
        Some(home) => {
            let home = home.to_string_lossy();
            match path.strip_prefix(home.as_ref()) {
                Some(rest) if rest.is_empty() || rest.starts_with('/') => format!("~{rest}"),
                _ => path.to_owned(),
            }
        }
        None => path.to_owned(),
    }
}

fn file_entry(
    letter: &'static str,
    path: &str,
    insertions: Option<i64>,
    deletions: Option<i64>,
    scope: DiffScope,
) -> InfoEntry {
    let mut spans = vec![
        Span::styled(format!("{letter} "), change_style(letter)),
        Span::raw(path.to_owned()),
    ];
    if let (Some(add), Some(del)) = (insertions, deletions)
        && add + del > 0
    {
        spans.push(Span::raw(" "));
        if add > 0 {
            spans.push(Span::styled(format!("+{add}"), ADDED));
        }
        if del > 0 {
            if add > 0 {
                spans.push(Span::raw(" "));
            }
            spans.push(Span::styled(format!("-{del}"), REMOVED));
        }
    }
    InfoEntry::item(
        spans,
        InfoTarget::File {
            path: path.to_owned(),
            scope,
        },
    )
}

fn tally(insertions: i64, deletions: i64, complete: bool) -> Vec<Span<'static>> {
    if !complete {
        return Vec::new();
    }
    vec![
        Span::raw("  "),
        Span::styled(format!("+{insertions}"), ADDED),
        Span::raw(" "),
        Span::styled(format!("-{deletions}"), REMOVED),
    ]
}

impl AppState {
    pub fn panel_thread(&self) -> Option<&str> {
        self.open_thread.as_deref()
    }

    pub fn panel_environment(&self) -> Option<&Environment> {
        self.snapshot
            .thread(self.panel_thread()?)?
            .environment
            .as_ref()
    }

    fn panel_environment_id(&self) -> Option<String> {
        let thread_id = self.panel_thread()?;
        self.snapshot
            .thread(thread_id)
            .and_then(|detail| detail.thread.environment_id.clone())
            .or_else(|| {
                self.sidebar_entry(thread_id)
                    .and_then(|entry| entry.thread.environment_id.clone())
            })
    }

    pub fn panel_merge_base(&self) -> Option<String> {
        let from_status = self
            .panel
            .workspace()
            .and_then(|status| status.merge_base.as_ref())
            .map(|merge_base| merge_base.merge_base_branch.clone())
            .filter(|branch| !branch.is_empty());
        from_status
            .or_else(|| self.panel.status_base.clone())
            .or_else(|| {
                self.panel_environment()
                    .and_then(Environment::resolved_merge_base_branch)
                    .map(str::to_owned)
            })
            .or_else(|| {
                self.panel
                    .workspace()
                    .map(|status| status.branch.default_branch.clone())
                    .filter(|branch| !branch.is_empty())
            })
    }

    fn status_request(&mut self, environment_id: &str, base: Option<String>) -> Effect {
        self.panel.status_fetching = true;
        self.panel.status_base = base.clone();
        Effect::Panel(PanelRequest::Status {
            environment_id: environment_id.to_owned(),
            merge_base_branch: base,
        })
    }

    pub fn sync_panel(&mut self) -> Vec<Effect> {
        let mut effects = self.sync_terminals();
        effects.extend(self.sync_workspace_panel());
        effects
    }

    fn sync_workspace_panel(&mut self) -> Vec<Effect> {
        if !self.panel_visible {
            return Vec::new();
        }
        let thread_id = self.panel_thread().map(str::to_owned);
        let environment_id = self.panel_environment_id();
        if self.panel.thread_id != thread_id || self.panel.environment_id != environment_id {
            self.panel.reset(thread_id, environment_id.clone());
            self.panel.seen_generation = environment_id
                .as_ref()
                .and_then(|id| self.snapshot.generations.environments.get(id).copied())
                .unwrap_or(0);
        }
        let Some(environment_id) = environment_id else {
            return Vec::new();
        };
        let mut effects = Vec::new();
        let generation = self
            .snapshot
            .generations
            .environments
            .get(&environment_id)
            .copied()
            .unwrap_or(0);
        let changed = generation != self.panel.seen_generation;
        self.panel.seen_generation = generation;
        let is_git = self
            .panel_environment()
            .map(|environment| environment.is_git_repo);
        if (self.panel.status.is_idle() && !self.panel.status_fetching) || changed {
            if self.panel.status.is_idle() {
                self.panel.status = Load::Loading;
            }
            let base = self
                .panel_environment()
                .and_then(Environment::resolved_merge_base_branch)
                .map(str::to_owned)
                .or_else(|| self.panel.status_base.clone());
            effects.push(self.status_request(&environment_id, base));
        }
        if is_git != Some(false) && (self.panel.pull_request.is_idle() || changed) {
            if self.panel.pull_request.is_idle() {
                self.panel.pull_request = Load::Loading;
            }
            effects.push(Effect::Panel(PanelRequest::PullRequest {
                environment_id: environment_id.clone(),
            }));
        }
        if changed {
            self.panel.diff_stale = true;
        }
        effects.extend(self.sync_diff());
        effects
    }

    fn sync_diff(&mut self) -> Vec<Effect> {
        if self.panel.tab != PanelTab::Diff {
            return Vec::new();
        }
        let Some(environment_id) = self.panel.environment_id.clone() else {
            return Vec::new();
        };
        if matches!(
            self.panel.status,
            Load::Loaded(StatusView::NotApplicable(_))
        ) {
            return Vec::new();
        }
        if should_reset_scope(&self.panel.scope, self.panel.workspace())
            && self.panel.status.loaded().is_some()
        {
            self.panel.scope = DiffScope::All;
        }
        let base = self.panel_merge_base();
        let status_settled =
            !self.panel.status_fetching || matches!(self.panel.status, Load::Loaded(_));
        if base.is_none() && !status_settled {
            return Vec::new();
        }
        let Some(target) = self.panel.scope.target(base.as_deref()) else {
            return Vec::new();
        };
        let reload = self.panel.diff_target.as_ref() != Some(&target)
            || self.panel.diff.is_idle()
            || self.panel.diff_stale;
        if !reload {
            return self.diff_loads();
        }
        if self.panel.diff_target.as_ref() != Some(&target) {
            self.panel.diff = Load::Loading;
        }
        self.panel.diff_stale = false;
        self.panel.diff_target = Some(target.clone());
        vec![Effect::Panel(PanelRequest::DiffFiles {
            environment_id,
            target,
        })]
    }

    fn diff_loads(&mut self) -> Vec<Effect> {
        let Some(environment_id) = self.panel.environment_id.clone() else {
            return Vec::new();
        };
        let Some(document) = self.panel.document_mut() else {
            return Vec::new();
        };
        let paths = document.pending_loads();
        if paths.is_empty() {
            return Vec::new();
        }
        vec![Effect::Panel(PanelRequest::DiffPatches {
            environment_id,
            target: document.target.clone(),
            paths,
        })]
    }

    pub fn panel_tick(&mut self) -> Vec<Effect> {
        if !self.panel_visible || self.panel.tab != PanelTab::Diff {
            return Vec::new();
        }
        self.diff_loads()
    }

    pub fn panel_loaded(&mut self, loaded: PanelLoaded) -> Vec<Effect> {
        match loaded {
            PanelLoaded::Status {
                environment_id,
                merge_base_branch,
                result,
            } => {
                if self.panel.environment_id.as_deref() != Some(environment_id.as_str()) {
                    return Vec::new();
                }
                self.panel.status_fetching = false;
                let view = match result {
                    Ok(EnvironmentStatusResponse::Available { workspace }) => {
                        StatusView::Available(workspace)
                    }
                    Ok(EnvironmentStatusResponse::NotApplicable { message, .. }) => {
                        StatusView::NotApplicable(message)
                    }
                    Ok(EnvironmentStatusResponse::Unavailable { failure }) => {
                        StatusView::Unavailable(failure.message)
                    }
                    Ok(EnvironmentStatusResponse::Unknown(_)) => {
                        StatusView::Unavailable("Unknown workspace status".into())
                    }
                    Err(error) => {
                        if self.panel.status.loaded().is_some() {
                            return Vec::new();
                        }
                        self.panel.status = Load::Failed(error);
                        return self.sync_diff();
                    }
                };
                let follow_up = match (&view, &merge_base_branch) {
                    (StatusView::Available(status), None)
                        if status.merge_base.is_none()
                            && !status.branch.default_branch.is_empty()
                            && status.branch.current_branch.as_deref()
                                != Some(status.branch.default_branch.as_str()) =>
                    {
                        Some(status.branch.default_branch.clone())
                    }
                    _ => None,
                };
                self.panel.status = Load::Loaded(view);
                let mut effects = Vec::new();
                if let Some(base) = follow_up {
                    effects.push(self.status_request(&environment_id, Some(base)));
                }
                effects.extend(self.sync_diff());
                effects
            }
            PanelLoaded::PullRequest {
                environment_id,
                result,
            } => {
                if self.panel.environment_id.as_deref() != Some(environment_id.as_str()) {
                    return Vec::new();
                }
                self.panel.pull_request = match result {
                    Ok(EnvironmentPullRequestResponse::Available { pull_request }) => {
                        Load::Loaded(Some(pull_request))
                    }
                    Ok(EnvironmentPullRequestResponse::Unavailable { message }) => {
                        Load::Failed(message)
                    }
                    Ok(_) => Load::Loaded(None),
                    Err(error) => Load::Failed(error),
                };
                Vec::new()
            }
            PanelLoaded::DiffFiles {
                environment_id,
                target,
                result,
            } => {
                if self.panel.environment_id.as_deref() != Some(environment_id.as_str())
                    || self.panel.diff_target.as_ref() != Some(&target)
                {
                    return Vec::new();
                }
                self.diff_files_loaded(target, result)
            }
            PanelLoaded::DiffPatches {
                environment_id,
                target,
                paths,
                result,
            } => {
                if self.panel.environment_id.as_deref() != Some(environment_id.as_str()) {
                    return Vec::new();
                }
                let Some(document) = self.panel.document_mut() else {
                    return Vec::new();
                };
                if document.target != target {
                    return Vec::new();
                }
                match result {
                    Ok(EnvironmentDiffPatchResponse::Available { patches }) => {
                        let returned: Vec<&String> =
                            patches.iter().map(|patch| &patch.path).collect();
                        let missing: Vec<String> = paths
                            .iter()
                            .filter(|path| !returned.contains(path))
                            .cloned()
                            .collect();
                        document.apply_patches(patches);
                        if !missing.is_empty() {
                            document.apply_patches(
                                missing
                                    .into_iter()
                                    .map(|path| DiffPatchEntry {
                                        path,
                                        patch: String::new(),
                                        truncated: false,
                                    })
                                    .collect(),
                            );
                        }
                    }
                    Ok(EnvironmentDiffPatchResponse::NotApplicable { message, .. }) => {
                        document.fail_patches(&paths, &message)
                    }
                    Ok(EnvironmentDiffPatchResponse::Unavailable { failure }) => {
                        document.fail_patches(&paths, &failure.message)
                    }
                    Ok(EnvironmentDiffPatchResponse::Unknown(_)) => {
                        document.fail_patches(&paths, "unknown response")
                    }
                    Err(error) => document.fail_patches(&paths, &error),
                }
                self.diff_loads()
            }
            PanelLoaded::File {
                environment_id,
                path,
                result,
            } => {
                if self.panel.environment_id.as_deref() != Some(environment_id.as_str()) {
                    return Vec::new();
                }
                let Some(preview) = self.panel.preview.as_mut() else {
                    return Vec::new();
                };
                if preview.path != path {
                    return Vec::new();
                }
                match result {
                    Ok(file) => {
                        let (lines, note) = preview_lines(&file);
                        preview.body = Load::Loaded(lines);
                        preview.note = note;
                        preview.positioned = false;
                        preview.position();
                    }
                    Err(error) => preview.body = Load::Failed(error),
                }
                Vec::new()
            }
        }
    }

    fn diff_files_loaded(
        &mut self,
        target: WorkspaceDiffTarget,
        result: Result<EnvironmentDiffFilesResponse, String>,
    ) -> Vec<Effect> {
        let listing = match result {
            Ok(EnvironmentDiffFilesResponse::Available {
                files,
                truncated,
                shortstat,
                merge_base_ref,
                initial_patches,
            }) => DiffListing {
                files,
                truncated,
                shortstat,
                merge_base_ref,
                initial_patches,
            },
            Ok(EnvironmentDiffFilesResponse::NotApplicable { message, .. }) => {
                self.panel.diff = Load::Failed(message);
                return Vec::new();
            }
            Ok(EnvironmentDiffFilesResponse::Unavailable { failure }) => {
                self.panel.diff = Load::Failed(failure.message);
                return Vec::new();
            }
            Ok(EnvironmentDiffFilesResponse::Unknown(_)) => {
                self.panel.diff = Load::Failed("Unknown diff response".into());
                return Vec::new();
            }
            Err(error) => {
                if self.panel.document().is_none() {
                    self.panel.diff = Load::Failed(error);
                }
                return Vec::new();
            }
        };
        let mut document = DiffDocument::new(self.panel.scope.clone(), target, listing);
        if let Some(previous) = self.panel.document()
            && previous.target == document.target
        {
            let cursor_path = previous
                .cursor_file()
                .map(|file| previous.files[file].entry.path.clone());
            for file in &mut document.files {
                if let Some(old) = previous
                    .files
                    .iter()
                    .find(|old| old.entry.path == file.entry.path)
                {
                    file.collapsed = old.collapsed;
                }
            }
            document.refresh_rows();
            document.viewport.set(previous.viewport.get());
            if let Some(path) = cursor_path
                && let Some(file) = document.file_index(&path)
                && let Some(row) = document.header_row(file)
            {
                document.set_cursor(row);
            }
        }
        let mut loads = Vec::new();
        if let Some(path) = self.panel.reveal.take()
            && let Some(load) = document.reveal(&path)
        {
            loads.push(load);
        }
        self.panel.diff = Load::Loaded(document);
        let mut effects = self.patch_request(loads);
        effects.extend(self.diff_loads());
        effects
    }

    fn patch_request(&self, paths: Vec<String>) -> Vec<Effect> {
        let (Some(environment_id), Some(document)) =
            (self.panel.environment_id.clone(), self.panel.document())
        else {
            return Vec::new();
        };
        if paths.is_empty() {
            return Vec::new();
        }
        vec![Effect::Panel(PanelRequest::DiffPatches {
            environment_id,
            target: document.target.clone(),
            paths,
        })]
    }

    pub fn set_panel_tab(&mut self, tab: PanelTab) -> Vec<Effect> {
        self.panel.tab = tab;
        self.panel_visible = true;
        self.sync_panel()
    }

    pub fn toggle_diff_panel(&mut self) -> Vec<Effect> {
        if self.panel_visible && self.panel.tab == PanelTab::Diff && self.focus == Focus::Panel {
            self.panel_visible = false;
            self.panel.maximized = false;
            self.focus = Focus::Main;
            return Vec::new();
        }
        self.focus = Focus::Panel;
        self.set_panel_tab(PanelTab::Diff)
    }

    pub fn cycle_panel_tab(&mut self, forward: bool) -> Vec<Effect> {
        let index = self.panel.tab.index();
        let count = PanelTab::ALL.len();
        let next = if forward {
            (index + 1) % count
        } else {
            (index + count - 1) % count
        };
        self.set_panel_tab(PanelTab::ALL[next])
    }

    pub fn toggle_panel_maximized(&mut self) {
        self.panel.maximized = !self.panel.maximized;
        if self.panel.maximized {
            self.panel_visible = true;
            self.focus = Focus::Panel;
        }
    }

    fn set_scope(&mut self, scope: DiffScope) -> Vec<Effect> {
        if self.panel.scope != scope {
            self.panel.scope = scope;
            self.panel.diff = Load::Idle;
            self.panel.diff_target = None;
        }
        self.set_panel_tab(PanelTab::Diff)
    }

    pub fn cycle_scope(&mut self, forward: bool) -> Vec<Effect> {
        let options = scope_options(self.panel.workspace());
        let index = options
            .iter()
            .position(|scope| *scope == self.panel.scope)
            .unwrap_or(0);
        let count = options.len();
        let next = if forward {
            (index + 1) % count
        } else {
            (index + count - 1) % count
        };
        let scope = options[next].clone();
        self.toast(format!("Diff: {}", scope.label()), ToastLevel::Info);
        self.set_scope(scope)
    }

    pub fn show_in_diff(&mut self, path: &str, scope: DiffScope) -> Vec<Effect> {
        self.panel.reveal = Some(path.to_owned());
        let same = self.panel.scope == scope && self.panel.document().is_some();
        let mut effects = self.set_scope(scope);
        if same {
            let path = self.panel.reveal.take().unwrap_or_default();
            let load = self
                .panel
                .document_mut()
                .and_then(|document| document.reveal(&path));
            effects.extend(self.patch_request(load.into_iter().collect()));
            effects.extend(self.diff_loads());
        }
        effects
    }

    pub fn open_preview(
        &mut self,
        path: &str,
        target: &WorkspaceDiffTarget,
        merge_base_ref: Option<&str>,
        side: DiffFileSide,
        line: Option<u32>,
    ) -> Vec<Effect> {
        let Some(environment_id) = self.panel.environment_id.clone() else {
            return Vec::new();
        };
        let Some(query) = diff_file_query(target, merge_base_ref, path, side) else {
            self.toast("The merge base is not known yet", ToastLevel::Info);
            return Vec::new();
        };
        let absolute = self.absolute_path(path);
        let label = match side {
            DiffFileSide::Old => format!("{path} (before)"),
            _ => path.to_owned(),
        };
        self.panel.preview_return = Some(self.panel.tab).filter(|tab| *tab != PanelTab::File);
        self.panel.preview = Some(FilePreview {
            path: path.to_owned(),
            label,
            absolute,
            line,
            body: Load::Loading,
            note: None,
            scroll: 0,
            column: 0,
            viewport: Cell::new(20),
            positioned: false,
        });
        self.panel.tab = PanelTab::File;
        self.panel_visible = true;
        vec![Effect::Panel(PanelRequest::File {
            environment_id,
            path: path.to_owned(),
            query,
        })]
    }

    fn absolute_path(&self, path: &str) -> Option<String> {
        let root = self.panel_environment()?.path.as_deref()?;
        Some(format!(
            "{}/{}",
            root.trim_end_matches('/'),
            path.trim_start_matches('/')
        ))
    }

    fn edit_effect(&mut self, path: &str, line: Option<u32>) -> Vec<Effect> {
        match self.absolute_path(path) {
            Some(path) => vec![Effect::EditFile { path, line }],
            None => {
                self.toast("The workspace path is not known", ToastLevel::Error);
                Vec::new()
            }
        }
    }

    pub fn info_entries(&self) -> Vec<InfoEntry> {
        let mut entries = Vec::new();
        let Some(thread_id) = self.panel_thread() else {
            return entries;
        };
        let detail = self.snapshot.thread(thread_id);
        let entry = self.sidebar_entry(thread_id);
        let thread = detail
            .map(|detail| &detail.thread)
            .or_else(|| entry.map(|entry| &entry.thread));
        let Some(thread) = thread else {
            return entries;
        };
        entries.push(InfoEntry::heading(self.thread_title(thread_id)));
        let status = entry
            .map(|entry| entry.runtime.display_status)
            .or_else(|| detail.map(|detail| detail.runtime.display_status));
        if let Some(status) = status {
            entries.push(InfoEntry::field(
                "Status",
                vec![Span::raw(
                    crate::indicator::display_status_label(status).to_owned(),
                )],
            ));
        }
        if let Some(name) = self.project_name(&thread.project_id) {
            entries.push(InfoEntry::field("Project", vec![Span::raw(name)]));
        }
        let environment = detail.and_then(|detail| detail.environment.as_ref());
        if let Some(environment) = environment {
            entries.push(InfoEntry::field(
                "Environment",
                vec![Span::raw(environment_label(environment))],
            ));
            let host = detail.and_then(|detail| detail.host.clone()).or_else(|| {
                self.snapshot.hosts.data.as_ref().and_then(|hosts| {
                    hosts
                        .iter()
                        .find(|host| host.id == environment.host_id)
                        .cloned()
                })
            });
            let host_name = host
                .as_ref()
                .map(|host| host.name.clone())
                .or_else(|| detail.and_then(|detail| detail.environment_host_name.clone()));
            if let Some(name) = host_name.filter(|name| !name.is_empty()) {
                let mut spans = vec![Span::raw(name)];
                if host.as_ref().is_some_and(|host| {
                    host.status == bb_wire::environment::HostStatus::Disconnected
                }) {
                    spans.push(Span::styled(" (offline)", MUTED));
                }
                entries.push(InfoEntry::field("Machine", spans));
            }
            if let Some(path) = environment.path.as_deref() {
                entries.push(
                    InfoEntry::field("Directory", vec![Span::styled(home_relative(path), MUTED)])
                        .with_target(InfoTarget::Copy(path.to_owned())),
                );
            }
        } else if thread.environment_id.is_none() {
            entries.push(InfoEntry::field(
                "Environment",
                vec![Span::styled("none", MUTED)],
            ));
        }
        let host_active = environment.is_none_or(|environment| {
            environment.host_lifecycle == bb_wire::environment::EnvironmentHostLifecycle::Active
        });
        let workspace = self.panel.workspace();
        if host_active {
            if let Some((label, value)) = workspace.and_then(checkout_label) {
                entries.push(
                    InfoEntry::field(&label, vec![Span::raw(value.clone())])
                        .with_target(InfoTarget::Copy(value)),
                );
            }
            if let Some(status) = workspace {
                let on_default = status.branch.current_branch.as_deref()
                    == Some(status.branch.default_branch.as_str());
                if let Some(merge_base) = &status.merge_base
                    && !merge_base.merge_base_branch.is_empty()
                {
                    if !on_default {
                        entries.push(InfoEntry::field(
                            "Merge base",
                            vec![Span::raw(merge_base.merge_base_branch.clone())],
                        ));
                    }
                    entries.push(InfoEntry::field(
                        "Git status",
                        vec![Span::raw(format_branch_comparison(
                            merge_base.ahead_count,
                            merge_base.behind_count,
                            &merge_base.merge_base_branch,
                        ))],
                    ));
                }
            }
            match &self.panel.status {
                Load::Loading => {
                    entries.push(InfoEntry::field(
                        "Git status",
                        vec![Span::styled("loading…", MUTED)],
                    ));
                }
                Load::Loaded(StatusView::Unavailable(message)) | Load::Failed(message) => {
                    entries.push(InfoEntry::field(
                        "Git status",
                        vec![Span::styled(
                            message.trim_end_matches('.').to_owned(),
                            WARNING,
                        )],
                    ));
                }
                _ => {}
            }
        }
        if let Some(Some(pull_request)) = self.panel.pull_request.loaded() {
            let (text, style) = pull_request_summary(pull_request);
            entries.push(
                InfoEntry::field("Pull request", vec![Span::styled(text, style)])
                    .with_target(InfoTarget::Url(pull_request.url.clone())),
            );
        }
        if thread.is_archived() {
            entries.push(InfoEntry::field(
                "Archived",
                vec![Span::styled("yes", WARNING)],
            ));
        }
        let mut provider = vec![Span::raw(thread.provider_id.clone())];
        if let Some(execution) = self.execution_state(thread_id) {
            let resolved = execution.resolve(self.model_fallback(thread_id));
            if !resolved.model_label.is_empty() {
                provider.push(Span::styled(" · ", MUTED));
                provider.push(Span::raw(resolved.model_label));
            }
            if resolved.reasoning.is_known() {
                provider.push(Span::styled(
                    format!(" · {}", resolved.reasoning.as_str()),
                    MUTED,
                ));
            }
        }
        entries.push(InfoEntry::field("Provider", provider));
        if let Some(usage) = self
            .timeline_head(thread_id)
            .and_then(|head| head.context_window_usage.as_ref())
            .filter(|usage| usage.model_context_window > 0.0)
        {
            let percent = usage.used_tokens / usage.model_context_window * 100.0;
            entries.push(InfoEntry::field(
                "Context",
                vec![Span::raw(format!(
                    "{}{:.0}% · {} of {} tokens",
                    if usage.estimated { "~" } else { "" },
                    percent,
                    compact_tokens(usage.used_tokens),
                    compact_tokens(usage.model_context_window)
                ))],
            ));
        }
        if thread.created_at > 0 {
            entries.push(InfoEntry::field(
                "Created",
                vec![Span::raw(format_time(thread.created_at, self.now_ms))],
            ));
        }
        if thread.updated_at > 0 {
            entries.push(InfoEntry::field(
                "Updated",
                vec![Span::raw(format_time(thread.updated_at, self.now_ms))],
            ));
        }
        if !host_active {
            return entries;
        }
        let Some(status) = workspace else {
            return entries;
        };
        if let Some(merge_base) = &status.merge_base {
            if !merge_base.commits.is_empty() {
                entries.push(InfoEntry::blank());
                entries.push(InfoEntry::heading(format!(
                    "Commits ({})",
                    merge_base.commits.len()
                )));
                for commit in &merge_base.commits {
                    let mut spans = vec![
                        Span::styled(format!("{} ", commit.short_sha), WARNING),
                        Span::raw(commit.subject.clone()),
                    ];
                    if commit.authored_at > 0 {
                        spans.push(Span::styled(
                            format!(
                                "  {}",
                                crate::time::compact_relative_time(commit.authored_at, self.now_ms)
                            ),
                            MUTED,
                        ));
                    }
                    entries.push(InfoEntry::item(
                        spans,
                        InfoTarget::Commit(commit.sha.clone()),
                    ));
                }
            }
            if !merge_base.files.is_empty() {
                entries.push(InfoEntry::blank());
                let mut heading = vec![Span::styled(
                    format!("Files changed ({})", merge_base.files.len()),
                    HEADING,
                )];
                heading.extend(tally(
                    merge_base.insertions,
                    merge_base.deletions,
                    merge_base.line_stats_complete,
                ));
                entries.push(InfoEntry {
                    label: None,
                    spans: heading,
                    target: None,
                    heading: true,
                });
                for file in &merge_base.files {
                    entries.push(file_entry(
                        change_letter(file.status),
                        &file.path,
                        file.insertions,
                        file.deletions,
                        DiffScope::Committed,
                    ));
                }
            }
        }
        let tree = &status.working_tree;
        if !tree.files.is_empty() {
            entries.push(InfoEntry::blank());
            let mut heading = vec![Span::styled(
                format!("Uncommitted files ({})", tree.files.len()),
                HEADING,
            )];
            heading.extend(tally(
                tree.insertions,
                tree.deletions,
                tree.line_stats_complete,
            ));
            entries.push(InfoEntry {
                label: None,
                spans: heading,
                target: None,
                heading: true,
            });
            for file in &tree.files {
                entries.push(file_entry(
                    change_letter(file.status),
                    &file.path,
                    file.insertions,
                    file.deletions,
                    DiffScope::Uncommitted,
                ));
            }
        }
        entries
    }

    pub fn available_git_actions(&self) -> Vec<GitAction> {
        let mut actions = Vec::new();
        let archived = self
            .panel_thread()
            .and_then(|thread_id| self.snapshot.thread(thread_id))
            .is_some_and(|detail| detail.thread.is_archived());
        if !archived
            && self
                .panel
                .workspace()
                .is_some_and(|status| status.working_tree.has_uncommitted_changes)
        {
            actions.push(GitAction::Commit);
        }
        if let Some(Some(pull_request)) = self.panel.pull_request.loaded() {
            match pull_request.state {
                PullRequestState::Draft => actions.push(GitAction::MarkReady),
                PullRequestState::Open => {
                    actions.push(GitAction::Merge);
                    actions.push(GitAction::ConvertToDraft);
                }
                _ => {}
            }
        }
        actions
    }

    pub fn git_command(&mut self, action: GitAction) -> Vec<Effect> {
        let mut effects = Vec::new();
        if !self.panel_visible {
            self.panel_visible = true;
            effects.extend(self.sync_panel());
        }
        effects.extend(self.request_git_action(action));
        effects
    }

    pub fn request_git_action(&mut self, action: GitAction) -> Vec<Effect> {
        let (Some(thread_id), Some(environment_id)) = (
            self.panel_thread().map(str::to_owned),
            self.panel.environment_id.clone(),
        ) else {
            self.toast("Open a thread with an environment first", ToastLevel::Info);
            return Vec::new();
        };
        let loading = match action {
            GitAction::Commit => matches!(self.panel.status, Load::Idle | Load::Loading),
            _ => matches!(self.panel.pull_request, Load::Idle | Load::Loading),
        };
        if loading && !self.available_git_actions().contains(&action) {
            self.toast(
                "Git status is still loading, try again in a moment",
                ToastLevel::Info,
            );
            return Vec::new();
        }
        if !self.available_git_actions().contains(&action) {
            let reason = match action {
                GitAction::Commit => "No uncommitted changes to commit",
                GitAction::MarkReady => "No draft pull request",
                GitAction::ConvertToDraft | GitAction::Merge => "No open pull request",
            };
            self.toast(reason, ToastLevel::Info);
            return Vec::new();
        }
        let mut lines = Vec::new();
        let branch = self
            .panel
            .workspace()
            .and_then(|status| status.branch.current_branch.clone())
            .unwrap_or_else(|| "the current branch".into());
        match action {
            GitAction::Commit => {
                let tree = self.panel.workspace().map(|status| &status.working_tree);
                let count = tree.map_or(0, |tree| tree.files.len());
                lines.push(format!(
                    "Commit {count} changed file{} on {branch}.",
                    if count == 1 { "" } else { "s" }
                ));
                lines.push("The server stages everything and writes the commit message.".into());
            }
            GitAction::MarkReady | GitAction::ConvertToDraft | GitAction::Merge => {
                if let Some(Some(pull_request)) = self.panel.pull_request.loaded() {
                    lines.push(format!("#{} {}", pull_request.number, pull_request.title));
                    lines.push(format!(
                        "{} → {}",
                        pull_request.head_ref_name, pull_request.base_ref_name
                    ));
                }
            }
        }
        self.panel.confirm = Some(GitConfirm {
            action,
            thread_id,
            environment_id,
            lines,
            method: 0,
        });
        Vec::new()
    }

    pub fn git_action_finished(
        &mut self,
        mutation: &Mutation,
        result: Result<MutationReply, String>,
    ) -> Vec<Effect> {
        let Mutation::EnvironmentAction { environment_id, .. } = mutation else {
            return Vec::new();
        };
        match result {
            Ok(MutationReply::EnvironmentAction(response)) => {
                let message = match *response {
                    EnvironmentActionResponse::Commit {
                        commit_sha,
                        commit_subject,
                        ..
                    } => format!(
                        "Committed {}: {commit_subject}",
                        commit_sha.chars().take(9).collect::<String>()
                    ),
                    EnvironmentActionResponse::PullRequestReady { message }
                    | EnvironmentActionResponse::PullRequestMerge { message, .. }
                    | EnvironmentActionResponse::PullRequestDraft { message }
                        if !message.is_empty() =>
                    {
                        message
                    }
                    _ => mutation.success_message().to_owned(),
                };
                self.toast(message, ToastLevel::Info);
            }
            Ok(_) => self.toast(mutation.success_message(), ToastLevel::Info),
            Err(error) => {
                self.toast(
                    format!("Failed to {}: {error}", mutation.failure_verb()),
                    ToastLevel::Error,
                );
                return Vec::new();
            }
        }
        if self.panel.environment_id.as_deref() != Some(environment_id.as_str()) {
            return Vec::new();
        }
        self.panel.diff_stale = true;
        self.panel.pull_request = Load::Idle;
        let base = self.panel.status_base.clone();
        let mut effects = vec![self.status_request(environment_id, base)];
        effects.extend(self.sync_panel());
        effects
    }
}

pub fn preview_lines(
    file: &EnvironmentDiffFileResponse,
) -> (Vec<Vec<Span<'static>>>, Option<String>) {
    use bb_wire::environment::ContentEncoding;
    let bytes = match file.content_encoding {
        ContentEncoding::Base64 => {
            let mime = file.mime_type.as_deref().unwrap_or("binary");
            let decoded = STANDARD.decode(file.content.as_bytes()).unwrap_or_default();
            if decoded.contains(&0) || std::str::from_utf8(&decoded).is_err() {
                return (
                    vec![vec![Span::styled(
                        format!("Binary file ({mime}, {} bytes)", file.size_bytes),
                        MUTED,
                    )]],
                    None,
                );
            }
            decoded
        }
        _ => file.content.clone().into_bytes(),
    };
    let truncated = bytes.len() > PREVIEW_MAX_BYTES;
    let shown = &bytes[..bytes.len().min(PREVIEW_MAX_BYTES)];
    let text = String::from_utf8_lossy(shown);
    let lines = crate::highlight::highlight_code(
        &text,
        crate::palette::extension(&file.path),
        Style::new(),
    );
    let note = truncated.then(|| {
        format!(
            "Showing the first {} KiB of {} KiB",
            PREVIEW_MAX_BYTES / 1024,
            bytes.len() / 1024
        )
    });
    (lines, note)
}

fn is_plain(key: &KeyEvent) -> bool {
    !key.modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
}

pub(crate) fn handle_confirm_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    let Some(mut confirm) = state.panel.confirm.take() else {
        return Vec::new();
    };
    match key.code {
        KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('q') => Vec::new(),
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Vec::new(),
        KeyCode::Left | KeyCode::Char('h') | KeyCode::BackTab
            if confirm.action == GitAction::Merge =>
        {
            confirm.method = (confirm.method + MERGE_METHODS.len() - 1) % MERGE_METHODS.len();
            state.panel.confirm = Some(confirm);
            Vec::new()
        }
        KeyCode::Right | KeyCode::Char('l') | KeyCode::Tab
            if confirm.action == GitAction::Merge =>
        {
            confirm.method = (confirm.method + 1) % MERGE_METHODS.len();
            state.panel.confirm = Some(confirm);
            Vec::new()
        }
        KeyCode::Char(digit @ '1'..='3') if confirm.action == GitAction::Merge => {
            confirm.method = usize::from(digit as u8 - b'1');
            state.panel.confirm = Some(confirm);
            Vec::new()
        }
        KeyCode::Enter | KeyCode::Char('y') => {
            if state.read_only {
                state.toast(
                    format!(
                        "Read-only mode: cannot {}",
                        confirm.action.title().to_lowercase()
                    ),
                    ToastLevel::Error,
                );
                return Vec::new();
            }
            let pending = match confirm.action {
                GitAction::Commit => "Creating commit…",
                GitAction::MarkReady => "Marking pull request ready…",
                GitAction::ConvertToDraft => "Converting pull request to draft…",
                GitAction::Merge => "Merging pull request…",
            };
            state.toast(pending, ToastLevel::Info);
            vec![Effect::Mutate(Mutation::EnvironmentAction {
                thread_id: confirm.thread_id.clone(),
                environment_id: confirm.environment_id.clone(),
                request: confirm.request(),
            })]
        }
        _ => {
            state.panel.confirm = Some(confirm);
            Vec::new()
        }
    }
}

const DIFF_KEYS: &[char] = &['n', 'N', '[', ']', 'a', 'y', 'p', 'e'];
const FILE_KEYS: &[char] = &['y', 'e'];

fn swallowed(key: &KeyEvent, keys: &[char]) -> bool {
    is_plain(key)
        && match key.code {
            KeyCode::Enter => true,
            KeyCode::Char(ch) => keys.contains(&ch),
            _ => false,
        }
}

enum Nav {
    Lines(isize),
    Page(isize, bool),
    Edge(bool),
    Pan(isize),
    PanHome,
}

fn nav_for(key: &KeyEvent) -> Option<Nav> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    Some(match key.code {
        KeyCode::Char('j') | KeyCode::Down if !ctrl => Nav::Lines(1),
        KeyCode::Char('k') | KeyCode::Up if !ctrl => Nav::Lines(-1),
        KeyCode::PageDown | KeyCode::Char(' ') if !ctrl => Nav::Page(1, false),
        KeyCode::PageUp => Nav::Page(-1, false),
        KeyCode::Char('f') if ctrl => Nav::Page(1, false),
        KeyCode::Char('b') if ctrl => Nav::Page(-1, false),
        KeyCode::Char('d') if ctrl => Nav::Page(1, true),
        KeyCode::Char('u') if ctrl => Nav::Page(-1, true),
        KeyCode::Char('g') | KeyCode::Home if !ctrl => Nav::Edge(true),
        KeyCode::Char('G') | KeyCode::End if !ctrl => Nav::Edge(false),
        KeyCode::Char('h') | KeyCode::Left if !ctrl => Nav::Pan(-8),
        KeyCode::Char('l') | KeyCode::Right if !ctrl => Nav::Pan(8),
        KeyCode::Char('0') if !ctrl => Nav::PanHome,
        _ => return None,
    })
}

pub(crate) fn handle_panel_key(state: &mut AppState, key: KeyEvent) -> Option<Vec<Effect>> {
    if state.panel.tab == PanelTab::Terminal && state.terminals.confirm_close.is_some() {
        return crate::terminal::handle_terminal_key(state, &key);
    }
    if is_plain(&key) {
        match key.code {
            KeyCode::Char(digit @ '1'..='4') => {
                let tab = PanelTab::ALL[usize::from(digit as u8 - b'1')];
                return Some(state.set_panel_tab(tab));
            }
            KeyCode::Char('C') => return Some(state.request_git_action(GitAction::Commit)),
            KeyCode::Char('R') => return Some(state.request_git_action(GitAction::MarkReady)),
            KeyCode::Char('M') => return Some(state.request_git_action(GitAction::Merge)),
            KeyCode::Char('W') => {
                return Some(state.request_git_action(GitAction::ConvertToDraft));
            }
            KeyCode::Char('r') => {
                if state.panel.tab == PanelTab::Terminal {
                    state.terminals.sessions = Load::Idle;
                    state.terminals.listing = false;
                }
                state.panel.status = Load::Idle;
                state.panel.status_fetching = false;
                state.panel.pull_request = Load::Idle;
                state.panel.diff_stale = true;
                return Some(state.sync_panel());
            }
            KeyCode::Esc => {
                if state.panel.tab == PanelTab::File
                    && let Some(tab) = state.panel.preview_return.take()
                {
                    state.panel.tab = tab;
                    return Some(Vec::new());
                }
                if state.panel.maximized {
                    state.panel.maximized = false;
                    return Some(Vec::new());
                }
                state.focus = Focus::Main;
                return Some(Vec::new());
            }
            _ => {}
        }
    }
    match state.panel.tab {
        PanelTab::Info => handle_info_key(state, &key),
        PanelTab::Diff => handle_diff_key(state, &key),
        PanelTab::File => handle_file_key(state, &key),
        PanelTab::Terminal => crate::terminal::handle_terminal_key(state, &key),
    }
}

fn handle_info_key(state: &mut AppState, key: &KeyEvent) -> Option<Vec<Effect>> {
    let entries = state.info_entries();
    let count = entries.len();
    if let Some(nav) = nav_for(key) {
        let viewport = state.panel.info_viewport.get().max(1) as isize;
        let cursor = state.panel.info_cursor as isize;
        let next = match nav {
            Nav::Lines(delta) => cursor + delta,
            Nav::Page(pages, half) => cursor + pages * if half { viewport / 2 } else { viewport },
            Nav::Edge(true) => 0,
            Nav::Edge(false) => count as isize - 1,
            Nav::Pan(_) | Nav::PanHome => return Some(Vec::new()),
        };
        state.panel.info_cursor = next.clamp(0, count.saturating_sub(1) as isize) as usize;
        let viewport = viewport as usize;
        if state.panel.info_cursor < state.panel.info_scroll {
            state.panel.info_scroll = state.panel.info_cursor;
        } else if state.panel.info_cursor >= state.panel.info_scroll + viewport {
            state.panel.info_scroll = state.panel.info_cursor + 1 - viewport;
        }
        return Some(Vec::new());
    }
    if !is_plain(key) {
        return None;
    }
    let target = entries
        .get(state.panel.info_cursor)
        .and_then(|entry| entry.target.clone());
    match key.code {
        KeyCode::Enter => Some(match target {
            Some(InfoTarget::Commit(sha)) => {
                let scope = state
                    .panel
                    .workspace()
                    .and_then(|status| status.merge_base.as_ref())
                    .and_then(|merge_base| merge_base.commits.iter().find(|c| c.sha == sha))
                    .map(DiffScope::from_commit);
                match scope {
                    Some(scope) => state.set_scope(scope),
                    None => Vec::new(),
                }
            }
            Some(InfoTarget::File { path, scope }) => state.show_in_diff(&path, scope),
            Some(InfoTarget::Copy(text)) => vec![Effect::Copy(text)],
            Some(InfoTarget::Url(url)) => vec![Effect::OpenExternal(url)],
            None => Vec::new(),
        }),
        KeyCode::Char('y') => Some(match target {
            Some(InfoTarget::Commit(text) | InfoTarget::Copy(text) | InfoTarget::Url(text)) => {
                vec![Effect::Copy(text)]
            }
            Some(InfoTarget::File { path, .. }) => vec![Effect::Copy(path)],
            None => Vec::new(),
        }),
        KeyCode::Char('o') => Some(match target {
            Some(InfoTarget::Url(url)) => vec![Effect::OpenExternal(url)],
            _ => Vec::new(),
        }),
        KeyCode::Char('p') => Some(match target {
            Some(InfoTarget::File { path, .. }) => {
                let deleted = state
                    .panel
                    .workspace()
                    .map(|status| {
                        status
                            .working_tree
                            .files
                            .iter()
                            .chain(status.merge_base.iter().flat_map(|base| base.files.iter()))
                            .any(|file| {
                                file.path == path && file.status == WorkspaceFileStatusKind::Deleted
                            })
                    })
                    .unwrap_or(false);
                let side = if deleted {
                    DiffFileSide::Old
                } else {
                    DiffFileSide::New
                };
                state.open_preview(&path, &WorkspaceDiffTarget::Uncommitted, None, side, None)
            }
            _ => Vec::new(),
        }),
        KeyCode::Char('e') => Some(match target {
            Some(InfoTarget::File { path, .. }) => state.edit_effect(&path, None),
            _ => Vec::new(),
        }),
        _ => None,
    }
}

fn handle_diff_key(state: &mut AppState, key: &KeyEvent) -> Option<Vec<Effect>> {
    if is_plain(key) {
        match key.code {
            KeyCode::Char('s') => return Some(state.cycle_scope(true)),
            KeyCode::Char('S') => return Some(state.cycle_scope(false)),
            _ => {}
        }
    }
    let Some(document) = state.panel.document_mut() else {
        return (nav_for(key).is_some() || swallowed(key, DIFF_KEYS)).then(Vec::new);
    };
    if let Some(nav) = nav_for(key) {
        match nav {
            Nav::Lines(delta) => document.move_cursor(delta),
            Nav::Page(pages, half) => document.page(pages, half),
            Nav::Edge(top) => document.set_cursor(if top { 0 } else { usize::MAX }),
            Nav::Pan(delta) => document.pan(delta),
            Nav::PanHome => document.column = 0,
        }
        return Some(state.diff_loads());
    }
    if !is_plain(key) {
        return None;
    }
    let row = document.row(document.cursor);
    match key.code {
        KeyCode::Char('n') => {
            if !document.jump_hunk(true) {
                state.toast("No more hunks below", ToastLevel::Info);
            }
            Some(state.diff_loads())
        }
        KeyCode::Char('N') => {
            if !document.jump_hunk(false) {
                state.toast("No more hunks above", ToastLevel::Info);
            }
            Some(state.diff_loads())
        }
        KeyCode::Char(']') => {
            document.jump_file(true);
            Some(state.diff_loads())
        }
        KeyCode::Char('[') => {
            document.jump_file(false);
            Some(state.diff_loads())
        }
        KeyCode::Char('a') => {
            let collapse = !document.all_collapsed();
            document.set_all_collapsed(collapse);
            Some(state.diff_loads())
        }
        KeyCode::Enter => {
            let Some(row) = row else {
                return Some(Vec::new());
            };
            let load = match row {
                DiffRow::Notice(file) => document
                    .load_on_demand(file)
                    .or_else(|| document.toggle_file(file)),
                DiffRow::Header(file) | DiffRow::Line(file, _) => document.toggle_file(file),
            };
            let mut effects = state.patch_request(load.into_iter().collect());
            effects.extend(state.diff_loads());
            Some(effects)
        }
        KeyCode::Char('y') => {
            let text = match row {
                Some(DiffRow::Line(file, line)) => document
                    .line(file, line)
                    .map(|line| line.text())
                    .unwrap_or_default(),
                Some(row) => document.files[row.file()].entry.path.clone(),
                None => return Some(Vec::new()),
            };
            Some(vec![Effect::Copy(text)])
        }
        KeyCode::Char('p') | KeyCode::Char('e') => {
            let Some(file) = row.map(DiffRow::file) else {
                return Some(Vec::new());
            };
            let entry = document.files[file].entry.clone();
            let line = document.cursor_line_number();
            let target = document.target.clone();
            let merge_base_ref = document.merge_base_ref.clone();
            let deleted = entry.change_kind == GitDiffFileChangeKind::Deleted;
            if key.code == KeyCode::Char('e') {
                if deleted {
                    state.toast("The file was deleted", ToastLevel::Info);
                    return Some(Vec::new());
                }
                return Some(state.edit_effect(&entry.path, line));
            }
            let side = if deleted {
                DiffFileSide::Old
            } else {
                DiffFileSide::New
            };
            Some(state.open_preview(&entry.path, &target, merge_base_ref.as_deref(), side, line))
        }
        _ => None,
    }
}

fn handle_file_key(state: &mut AppState, key: &KeyEvent) -> Option<Vec<Effect>> {
    let Some(preview) = state.panel.preview.as_mut() else {
        return (nav_for(key).is_some() || swallowed(key, FILE_KEYS)).then(Vec::new);
    };
    if let Some(nav) = nav_for(key) {
        let viewport = preview.viewport.get().max(1) as isize;
        match nav {
            Nav::Lines(delta) => preview.scroll_by(delta),
            Nav::Page(pages, half) => {
                preview.scroll_by(pages * if half { viewport / 2 } else { viewport })
            }
            Nav::Edge(true) => preview.scroll = 0,
            Nav::Edge(false) => preview.scroll_by(isize::MAX / 2),
            Nav::Pan(delta) => preview.column = preview.column.saturating_add_signed(delta),
            Nav::PanHome => preview.column = 0,
        }
        return Some(Vec::new());
    }
    if !is_plain(key) {
        return None;
    }
    match key.code {
        KeyCode::Char('y') => Some(vec![Effect::Copy(
            preview
                .absolute
                .clone()
                .unwrap_or_else(|| preview.path.clone()),
        )]),
        KeyCode::Char('e') => {
            let path = preview.path.clone();
            let line = preview
                .line
                .or_else(|| u32::try_from(preview.scroll + 1).ok());
            Some(state.edit_effect(&path, line))
        }
        _ => None,
    }
}
