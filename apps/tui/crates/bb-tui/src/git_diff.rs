use std::cell::Cell;
use std::collections::HashMap;

use bb_view::patch::{PatchLineKind, parse_unified_patch};
use bb_wire::environment::{
    DiffFileEntry, DiffFileLoadMode, DiffPatchEntry, EnvironmentDiffFileQuery,
    EnvironmentDiffQuery, GitDiffFileChangeKind, WorkspaceCommitSummary, WorkspaceDiffTarget,
    WorkspaceStatus,
};
use ratatui::style::{Color, Style};
use ratatui::text::Span;

pub const AUTO_COLLAPSE_FILE_THRESHOLD: usize = 10;
pub const PATCH_BATCH_LIMIT: usize = 50;
pub const PREFETCH_ROWS: usize = 60;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum DiffScope {
    #[default]
    All,
    Committed,
    Uncommitted,
    Commit {
        sha: String,
        short_sha: String,
        subject: String,
    },
}

impl DiffScope {
    pub fn label(&self) -> String {
        match self {
            Self::All => "All changes".into(),
            Self::Committed => "Committed changes".into(),
            Self::Uncommitted => "Uncommitted changes".into(),
            Self::Commit {
                short_sha, subject, ..
            } => format!("{short_sha} {subject}"),
        }
    }

    pub fn from_commit(commit: &WorkspaceCommitSummary) -> Self {
        Self::Commit {
            sha: commit.sha.clone(),
            short_sha: commit.short_sha.clone(),
            subject: commit.subject.clone(),
        }
    }

    pub fn target(&self, merge_base_branch: Option<&str>) -> Option<WorkspaceDiffTarget> {
        let base = merge_base_branch.filter(|branch| !branch.is_empty());
        match self {
            Self::Uncommitted => Some(WorkspaceDiffTarget::Uncommitted),
            Self::Committed => base.map(|branch| WorkspaceDiffTarget::BranchCommitted {
                merge_base_branch: branch.to_owned(),
            }),
            Self::Commit { sha, .. } => Some(WorkspaceDiffTarget::Commit { sha: sha.clone() }),
            Self::All => Some(match base {
                Some(branch) => WorkspaceDiffTarget::All {
                    merge_base_branch: branch.to_owned(),
                },
                None => WorkspaceDiffTarget::Uncommitted,
            }),
        }
    }
}

pub fn diff_query(target: &WorkspaceDiffTarget) -> EnvironmentDiffQuery {
    match target {
        WorkspaceDiffTarget::Uncommitted => EnvironmentDiffQuery::Uncommitted,
        WorkspaceDiffTarget::BranchCommitted { merge_base_branch } => {
            EnvironmentDiffQuery::BranchCommitted {
                merge_base_branch: merge_base_branch.clone(),
            }
        }
        WorkspaceDiffTarget::All { merge_base_branch } => EnvironmentDiffQuery::All {
            merge_base_branch: merge_base_branch.clone(),
        },
        WorkspaceDiffTarget::Commit { sha } => EnvironmentDiffQuery::Commit { sha: sha.clone() },
    }
}

pub fn diff_file_query(
    target: &WorkspaceDiffTarget,
    merge_base_ref: Option<&str>,
    path: &str,
    side: bb_wire::environment::DiffFileSide,
) -> Option<EnvironmentDiffFileQuery> {
    let path = path.to_owned();
    Some(match target {
        WorkspaceDiffTarget::Uncommitted => EnvironmentDiffFileQuery::Uncommitted { path, side },
        WorkspaceDiffTarget::BranchCommitted { .. } => EnvironmentDiffFileQuery::BranchCommitted {
            merge_base_ref: merge_base_ref?.to_owned(),
            path,
            side,
        },
        WorkspaceDiffTarget::All { .. } => EnvironmentDiffFileQuery::All {
            merge_base_ref: merge_base_ref?.to_owned(),
            path,
            side,
        },
        WorkspaceDiffTarget::Commit { sha } => EnvironmentDiffFileQuery::Commit {
            sha: sha.clone(),
            path,
            side,
        },
    })
}

fn merge_base_commits(status: Option<&WorkspaceStatus>) -> &[WorkspaceCommitSummary] {
    status
        .and_then(|status| status.merge_base.as_ref())
        .map(|merge_base| merge_base.commits.as_slice())
        .unwrap_or_default()
}

fn has_uncommitted(status: Option<&WorkspaceStatus>) -> bool {
    status.is_some_and(|status| !status.working_tree.files.is_empty())
}

pub fn scope_options(status: Option<&WorkspaceStatus>) -> Vec<DiffScope> {
    let commits = merge_base_commits(status);
    let uncommitted = has_uncommitted(status);
    let mut options = vec![DiffScope::All];
    if commits.is_empty() && !uncommitted {
        return options;
    }
    if !commits.is_empty() {
        options.push(DiffScope::Committed);
    }
    if uncommitted {
        options.push(DiffScope::Uncommitted);
    }
    options.extend(commits.iter().map(DiffScope::from_commit));
    options
}

pub fn should_reset_scope(scope: &DiffScope, status: Option<&WorkspaceStatus>) -> bool {
    let commits = merge_base_commits(status);
    match scope {
        DiffScope::All => false,
        DiffScope::Committed => commits.is_empty(),
        DiffScope::Uncommitted => !has_uncommitted(status),
        DiffScope::Commit { sha, .. } => !commits.iter().any(|commit| &commit.sha == sha),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Hunk,
    Context,
    Added,
    Removed,
    NoNewline,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiffLine {
    pub kind: LineKind,
    pub old: Option<u32>,
    pub new: Option<u32>,
    pub spans: Vec<Span<'static>>,
}

impl DiffLine {
    pub fn text(&self) -> String {
        self.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RenderedPatch {
    pub lines: Vec<DiffLine>,
    pub truncated: bool,
    pub binary: bool,
    pub number_width: usize,
}

pub const HUNK_STYLE: Style = Style::new().fg(Color::Cyan);
pub const NO_NEWLINE_STYLE: Style = Style::new().fg(Color::DarkGray);

pub fn render_patch(path: &str, patch: &str, truncated: bool) -> RenderedPatch {
    let files = parse_unified_patch(patch);
    let binary = files.iter().any(|file| file.is_binary);
    let mut lines = Vec::new();
    let mut code_lines: Vec<(usize, &str)> = Vec::new();
    let mut max_number = 0u32;
    for file in &files {
        for hunk in &file.hunks {
            lines.push(DiffLine {
                kind: LineKind::Hunk,
                old: None,
                new: None,
                spans: vec![Span::styled(hunk.header.clone(), HUNK_STYLE)],
            });
            for line in &hunk.lines {
                let kind = match line.kind {
                    PatchLineKind::Context => LineKind::Context,
                    PatchLineKind::Added => LineKind::Added,
                    PatchLineKind::Removed => LineKind::Removed,
                    PatchLineKind::NoNewline => LineKind::NoNewline,
                };
                max_number = max_number
                    .max(line.old_line.unwrap_or(0))
                    .max(line.new_line.unwrap_or(0));
                if kind == LineKind::NoNewline {
                    lines.push(DiffLine {
                        kind,
                        old: None,
                        new: None,
                        spans: vec![Span::styled(line.text.clone(), NO_NEWLINE_STYLE)],
                    });
                    continue;
                }
                code_lines.push((lines.len(), line.text.as_str()));
                lines.push(DiffLine {
                    kind,
                    old: line.old_line,
                    new: line.new_line,
                    spans: Vec::new(),
                });
            }
        }
    }
    let code = code_lines
        .iter()
        .map(|(_, text)| *text)
        .collect::<Vec<_>>()
        .join("\n");
    let language = crate::palette::extension(path);
    let mut highlighted = crate::highlight::highlight_code(&code, language, Style::new());
    if highlighted.len() != code_lines.len() {
        highlighted = code_lines
            .iter()
            .map(|(_, text)| vec![Span::raw(text.replace('\t', crate::wrap::TAB))])
            .collect();
    }
    for ((index, _), spans) in code_lines.iter().zip(highlighted) {
        lines[*index].spans = spans;
    }
    RenderedPatch {
        lines,
        truncated,
        binary,
        number_width: max_number.max(1).to_string().len(),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PatchState {
    Idle,
    Loading,
    Loaded(RenderedPatch),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiffFile {
    pub entry: DiffFileEntry,
    pub collapsed: bool,
    pub patch: PatchState,
}

impl DiffFile {
    pub fn status_letter(&self) -> &'static str {
        match self.entry.change_kind {
            GitDiffFileChangeKind::Added => "A",
            GitDiffFileChangeKind::Modified => "M",
            GitDiffFileChangeKind::Deleted => "D",
            GitDiffFileChangeKind::Renamed => "R",
            GitDiffFileChangeKind::Copied => "C",
            GitDiffFileChangeKind::TypeChanged => "T",
            GitDiffFileChangeKind::Unknown => "?",
        }
    }

    pub fn notice(&self) -> Option<String> {
        let changed = self.entry.additions + self.entry.deletions;
        match &self.patch {
            PatchState::Loading => Some("Loading diff…".into()),
            PatchState::Failed(error) => Some(format!("Failed to load diff: {error}")),
            PatchState::Idle => match self.entry.load_mode {
                DiffFileLoadMode::TooLarge => Some(format!(
                    "Diff too large to show ({changed} changed lines) · p preview file"
                )),
                DiffFileLoadMode::OnDemand if self.entry.binary => {
                    Some("Binary file · Enter to load".into())
                }
                DiffFileLoadMode::OnDemand => Some(format!(
                    "Large diff ({changed} changed lines) · Enter to load"
                )),
                _ => Some("Loading diff…".into()),
            },
            PatchState::Loaded(patch) if patch.lines.is_empty() && patch.binary => {
                Some("Binary file changed".into())
            }
            PatchState::Loaded(patch) if patch.lines.is_empty() => {
                Some("No textual changes".into())
            }
            PatchState::Loaded(patch) if patch.truncated => {
                Some("Patch truncated by the server".into())
            }
            PatchState::Loaded(_) => None,
        }
    }

    fn body_rows(&self) -> usize {
        if self.collapsed {
            return 0;
        }
        let lines = match &self.patch {
            PatchState::Loaded(patch) => patch.lines.len(),
            _ => 0,
        };
        lines + usize::from(self.notice().is_some())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffRow {
    Header(usize),
    Line(usize, usize),
    Notice(usize),
}

impl DiffRow {
    pub fn file(self) -> usize {
        match self {
            Self::Header(file) | Self::Line(file, _) | Self::Notice(file) => file,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiffDocument {
    pub scope: DiffScope,
    pub target: WorkspaceDiffTarget,
    pub merge_base_ref: Option<String>,
    pub shortstat: String,
    pub truncated: bool,
    pub files: Vec<DiffFile>,
    starts: Vec<usize>,
    total: usize,
    pub cursor: usize,
    pub scroll: usize,
    pub column: usize,
    pub viewport: Cell<usize>,
}

pub struct DiffListing {
    pub files: Vec<DiffFileEntry>,
    pub truncated: bool,
    pub shortstat: String,
    pub merge_base_ref: Option<String>,
    pub initial_patches: Vec<DiffPatchEntry>,
}

impl DiffDocument {
    pub fn new(scope: DiffScope, target: WorkspaceDiffTarget, listing: DiffListing) -> Self {
        let count = listing.files.len();
        let files = listing
            .files
            .into_iter()
            .map(|entry| DiffFile {
                collapsed: count > AUTO_COLLAPSE_FILE_THRESHOLD
                    || entry.change_kind == GitDiffFileChangeKind::Deleted,
                entry,
                patch: PatchState::Idle,
            })
            .collect();
        let mut document = Self {
            scope,
            target,
            merge_base_ref: listing.merge_base_ref,
            shortstat: listing.shortstat,
            truncated: listing.truncated,
            files,
            starts: Vec::new(),
            total: 0,
            cursor: 0,
            scroll: 0,
            column: 0,
            viewport: Cell::new(20),
        };
        document.apply_patches(listing.initial_patches);
        document
    }

    pub fn refresh_rows(&mut self) {
        self.rebuild();
    }

    pub fn row_count(&self) -> usize {
        self.total
    }

    fn rebuild(&mut self) {
        let anchor = self.row(self.cursor);
        self.starts.clear();
        let mut total = 0;
        for file in &self.files {
            self.starts.push(total);
            total += 1 + file.body_rows();
        }
        self.total = total;
        if let Some(anchor) = anchor {
            self.cursor = self.position_of(anchor);
        }
        self.cursor = self.cursor.min(total.saturating_sub(1));
        self.clamp_scroll();
    }

    fn position_of(&self, row: DiffRow) -> usize {
        let file = row.file();
        let Some(start) = self.starts.get(file).copied() else {
            return 0;
        };
        let body = self.files[file].body_rows();
        match row {
            DiffRow::Header(_) => start,
            DiffRow::Line(_, line) if body > 0 => {
                let lines = match &self.files[file].patch {
                    PatchState::Loaded(patch) => patch.lines.len(),
                    _ => 0,
                };
                if line < lines {
                    start + 1 + line
                } else {
                    start
                }
            }
            DiffRow::Notice(_) if body > 0 => start + body,
            _ => start,
        }
    }

    pub fn row(&self, index: usize) -> Option<DiffRow> {
        if index >= self.total {
            return None;
        }
        let file = self.starts.partition_point(|start| *start <= index) - 1;
        let offset = index - self.starts[file];
        if offset == 0 {
            return Some(DiffRow::Header(file));
        }
        let lines = match &self.files[file].patch {
            PatchState::Loaded(patch) if !self.files[file].collapsed => patch.lines.len(),
            _ => 0,
        };
        if offset <= lines {
            Some(DiffRow::Line(file, offset - 1))
        } else {
            Some(DiffRow::Notice(file))
        }
    }

    pub fn header_row(&self, file: usize) -> Option<usize> {
        self.starts.get(file).copied()
    }

    pub fn line(&self, file: usize, line: usize) -> Option<&DiffLine> {
        match &self.files.get(file)?.patch {
            PatchState::Loaded(patch) => patch.lines.get(line),
            _ => None,
        }
    }

    pub fn number_width(&self, file: usize) -> usize {
        match &self.files.get(file).map(|file| &file.patch) {
            Some(PatchState::Loaded(patch)) => patch.number_width,
            _ => 1,
        }
    }

    pub fn file_index(&self, path: &str) -> Option<usize> {
        self.files.iter().position(|file| file.entry.path == path)
    }

    pub fn cursor_file(&self) -> Option<usize> {
        self.row(self.cursor).map(DiffRow::file)
    }

    pub fn cursor_line_number(&self) -> Option<u32> {
        match self.row(self.cursor)? {
            DiffRow::Line(file, line) => {
                let patch = match &self.files[file].patch {
                    PatchState::Loaded(patch) => patch,
                    _ => return None,
                };
                patch.lines[line..]
                    .iter()
                    .find_map(|line| line.new)
                    .or_else(|| patch.lines[..line].iter().rev().find_map(|line| line.new))
            }
            _ => None,
        }
    }

    fn clamp_scroll(&mut self) {
        let viewport = self.viewport.get().max(1);
        let max_scroll = self.total.saturating_sub(viewport);
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        } else if self.cursor >= self.scroll + viewport {
            self.scroll = self.cursor + 1 - viewport;
        }
        self.scroll = self.scroll.min(max_scroll);
    }

    pub fn set_cursor(&mut self, index: usize) {
        self.cursor = index.min(self.total.saturating_sub(1));
        self.clamp_scroll();
    }

    pub fn move_cursor(&mut self, delta: isize) {
        self.set_cursor(self.cursor.saturating_add_signed(delta));
    }

    pub fn page(&mut self, pages: isize, half: bool) {
        let viewport = self.viewport.get().max(1);
        let step = if half { viewport / 2 } else { viewport }.max(1) as isize;
        self.move_cursor(step * pages);
    }

    pub fn scroll_to_cursor_top(&mut self) {
        let viewport = self.viewport.get().max(1);
        self.scroll = self.cursor.min(self.total.saturating_sub(viewport));
    }

    pub fn pan(&mut self, delta: isize) {
        self.column = self.column.saturating_add_signed(delta);
    }

    pub fn jump_file(&mut self, forward: bool) -> bool {
        let Some(current) = self.cursor_file() else {
            return false;
        };
        let on_header = matches!(self.row(self.cursor), Some(DiffRow::Header(_)));
        let target = if forward {
            current + 1
        } else if on_header {
            match current.checked_sub(1) {
                Some(previous) => previous,
                None => return false,
            }
        } else {
            current
        };
        let Some(row) = self.header_row(target) else {
            return false;
        };
        self.set_cursor(row);
        self.scroll_to_cursor_top();
        true
    }

    pub fn jump_hunk(&mut self, forward: bool) -> bool {
        let is_hunk = |document: &Self, index: usize| {
            matches!(
                document.row(index),
                Some(DiffRow::Line(file, line))
                    if document.line(file, line).is_some_and(|line| line.kind == LineKind::Hunk)
            )
        };
        let found = if forward {
            (self.cursor + 1..self.total).find(|index| is_hunk(self, *index))
        } else {
            (0..self.cursor).rev().find(|index| is_hunk(self, *index))
        };
        match found {
            Some(index) => {
                self.set_cursor(index);
                self.scroll_to_cursor_top();
                true
            }
            None => false,
        }
    }

    pub fn toggle_file(&mut self, file: usize) -> Option<String> {
        let target = self.files.get_mut(file)?;
        target.collapsed = !target.collapsed;
        let load = (!target.collapsed
            && target.patch == PatchState::Idle
            && target.entry.load_mode == DiffFileLoadMode::OnDemand)
            .then(|| {
                target.patch = PatchState::Loading;
                target.entry.path.clone()
            });
        let header = self.starts.get(file).copied().unwrap_or(0);
        self.rebuild();
        if self.files[file].collapsed {
            self.set_cursor(header);
        }
        load
    }

    pub fn load_on_demand(&mut self, file: usize) -> Option<String> {
        let target = self.files.get_mut(file)?;
        if target.patch != PatchState::Idle || target.entry.load_mode == DiffFileLoadMode::TooLarge
        {
            return None;
        }
        target.collapsed = false;
        target.patch = PatchState::Loading;
        let path = target.entry.path.clone();
        self.rebuild();
        Some(path)
    }

    pub fn set_all_collapsed(&mut self, collapsed: bool) {
        for file in &mut self.files {
            file.collapsed = collapsed;
        }
        self.rebuild();
    }

    pub fn all_collapsed(&self) -> bool {
        !self.files.is_empty() && self.files.iter().all(|file| file.collapsed)
    }

    pub fn pending_loads(&mut self) -> Vec<String> {
        let viewport = self.viewport.get().max(1);
        let from = self.scroll.saturating_sub(PREFETCH_ROWS);
        let to = self.scroll + viewport + PREFETCH_ROWS;
        let first = self.starts.partition_point(|start| *start < from);
        let first = first.saturating_sub(1);
        let mut paths = Vec::new();
        for index in first..self.files.len() {
            if self.starts[index] > to || paths.len() >= PATCH_BATCH_LIMIT {
                break;
            }
            let file = &mut self.files[index];
            if !file.collapsed
                && file.patch == PatchState::Idle
                && file.entry.load_mode == DiffFileLoadMode::Auto
            {
                file.patch = PatchState::Loading;
                paths.push(file.entry.path.clone());
            }
        }
        if !paths.is_empty() {
            self.rebuild();
        }
        paths
    }

    pub fn apply_patches(&mut self, patches: Vec<DiffPatchEntry>) {
        let index: HashMap<String, usize> = self
            .files
            .iter()
            .enumerate()
            .map(|(position, file)| (file.entry.path.clone(), position))
            .collect();
        for patch in patches {
            if let Some(position) = index.get(&patch.path) {
                let rendered = render_patch(&patch.path, &patch.patch, patch.truncated);
                self.files[*position].patch = PatchState::Loaded(rendered);
            }
        }
        self.rebuild();
    }

    pub fn fail_patches(&mut self, paths: &[String], error: &str) {
        for file in &mut self.files {
            if paths.contains(&file.entry.path) && file.patch == PatchState::Loading {
                file.patch = PatchState::Failed(error.to_owned());
            }
        }
        self.rebuild();
    }

    pub fn reveal(&mut self, path: &str) -> Option<String> {
        let file = self.file_index(path)?;
        let mut load = None;
        if self.files[file].collapsed {
            load = self.toggle_file(file);
        }
        if let Some(row) = self.header_row(file) {
            self.set_cursor(row);
            self.scroll_to_cursor_top();
        }
        load
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bb_wire::environment::{
        DiffFileOrigin, WorkspaceFileStatus, WorkspaceFileStatusKind, WorkspaceMergeBase,
    };

    fn entry(path: &str, additions: u64, mode: DiffFileLoadMode) -> DiffFileEntry {
        DiffFileEntry {
            path: path.into(),
            previous_path: None,
            change_kind: GitDiffFileChangeKind::Modified,
            additions,
            deletions: 0,
            binary: false,
            origin: DiffFileOrigin::Tracked,
            load_mode: mode,
        }
    }

    fn patch(path: &str, hunks: usize) -> DiffPatchEntry {
        let mut text = format!("diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n");
        for hunk in 0..hunks {
            let start = hunk * 10 + 1;
            text.push_str(&format!("@@ -{start},2 +{start},3 @@ fn x\n a\n+b\n c\n"));
        }
        DiffPatchEntry {
            path: path.into(),
            patch: text,
            truncated: false,
        }
    }

    fn listing(files: Vec<DiffFileEntry>, initial: Vec<DiffPatchEntry>) -> DiffListing {
        DiffListing {
            files,
            truncated: false,
            shortstat: "2 files changed".into(),
            merge_base_ref: Some("abc123".into()),
            initial_patches: initial,
        }
    }

    #[test]
    fn rendered_patch_keeps_line_numbers_and_highlights() {
        let rendered = render_patch(
            "src/lib.rs",
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -9,2 +9,3 @@\n fn a() {}\n+let x = 1;\n-old\n\\ No newline at end of file\n",
            false,
        );
        let kinds: Vec<LineKind> = rendered.lines.iter().map(|line| line.kind).collect();
        assert_eq!(
            kinds,
            [
                LineKind::Hunk,
                LineKind::Context,
                LineKind::Added,
                LineKind::Removed,
                LineKind::NoNewline
            ]
        );
        assert_eq!(rendered.lines[1].old, Some(9));
        assert_eq!(rendered.lines[2].new, Some(10));
        assert_eq!(rendered.lines[3].old, Some(10));
        assert_eq!(rendered.lines[2].text(), "let x = 1;");
        assert!(rendered.lines[2].spans.len() > 1);
        assert_eq!(rendered.number_width, 2);
    }

    #[test]
    fn many_files_start_collapsed_and_only_visible_auto_files_load() {
        let files: Vec<DiffFileEntry> = (0..120)
            .map(|index| {
                let mode = if index == 1 {
                    DiffFileLoadMode::OnDemand
                } else {
                    DiffFileLoadMode::Auto
                };
                entry(&format!("f{index}.rs"), 3, mode)
            })
            .collect();
        let mut document = DiffDocument::new(
            DiffScope::All,
            WorkspaceDiffTarget::Uncommitted,
            listing(files, Vec::new()),
        );
        assert_eq!(document.row_count(), 120);
        assert!(document.pending_loads().is_empty());
        document.set_all_collapsed(false);
        document.viewport.set(10);
        let first = document.pending_loads();
        assert!(first.len() <= PATCH_BATCH_LIMIT, "{}", first.len());
        assert!(!first.contains(&"f1.rs".to_owned()));
        assert_eq!(first[0], "f0.rs");
        assert!(document.pending_loads().len() < first.len() || first.len() == PATCH_BATCH_LIMIT);
        assert!(!first.contains(&"f119.rs".to_owned()));
        let load = document.load_on_demand(1);
        assert_eq!(load.as_deref(), Some("f1.rs"));
        assert_eq!(document.load_on_demand(1), None);
    }

    #[test]
    fn hunk_and_file_navigation_over_loaded_patches() {
        let files = vec![
            entry("a.rs", 2, DiffFileLoadMode::Auto),
            entry("b.rs", 2, DiffFileLoadMode::Auto),
        ];
        let mut document = DiffDocument::new(
            DiffScope::All,
            WorkspaceDiffTarget::Uncommitted,
            listing(files, vec![patch("a.rs", 2), patch("b.rs", 1)]),
        );
        assert_eq!(document.row_count(), 2 + 8 + 4);
        assert_eq!(document.row(0), Some(DiffRow::Header(0)));
        assert!(document.jump_hunk(true));
        assert_eq!(document.row(document.cursor), Some(DiffRow::Line(0, 0)));
        assert!(document.jump_hunk(true));
        assert_eq!(document.row(document.cursor), Some(DiffRow::Line(0, 4)));
        assert!(document.jump_hunk(true));
        assert_eq!(document.row(document.cursor), Some(DiffRow::Line(1, 0)));
        assert!(!document.jump_hunk(true));
        assert!(document.jump_hunk(false));
        assert_eq!(document.row(document.cursor), Some(DiffRow::Line(0, 4)));
        document.move_cursor(2);
        assert_eq!(document.cursor_line_number(), Some(12));
        assert!(document.jump_file(false));
        assert_eq!(document.row(document.cursor), Some(DiffRow::Header(0)));
        assert!(document.jump_file(true));
        assert_eq!(document.row(document.cursor), Some(DiffRow::Header(1)));
        document.toggle_file(1);
        assert_eq!(document.row_count(), 2 + 8);
        assert_eq!(document.row(document.cursor), Some(DiffRow::Header(1)));
    }

    #[test]
    fn notices_describe_unloaded_files() {
        let mut too_large = entry("big.json", 30_000, DiffFileLoadMode::TooLarge);
        too_large.deletions = 5;
        let document = DiffDocument::new(
            DiffScope::All,
            WorkspaceDiffTarget::Uncommitted,
            listing(vec![too_large], Vec::new()),
        );
        assert_eq!(document.row_count(), 2);
        assert_eq!(document.row(1), Some(DiffRow::Notice(0)));
        assert!(
            document.files[0]
                .notice()
                .unwrap()
                .contains("30005 changed lines")
        );
    }

    fn status(commits: &[&str], uncommitted: bool) -> WorkspaceStatus {
        let mut status = WorkspaceStatus::default();
        if uncommitted {
            status.working_tree.files.push(WorkspaceFileStatus {
                path: "a".into(),
                status: WorkspaceFileStatusKind::Modified,
                ..WorkspaceFileStatus::default()
            });
        }
        status.merge_base = Some(WorkspaceMergeBase {
            merge_base_branch: "main".into(),
            commits: commits
                .iter()
                .map(|sha| WorkspaceCommitSummary {
                    sha: (*sha).into(),
                    short_sha: sha[..3].into(),
                    subject: format!("commit {sha}"),
                    ..WorkspaceCommitSummary::default()
                })
                .collect(),
            ..WorkspaceMergeBase::default()
        });
        status
    }

    #[test]
    fn scope_options_follow_the_web_selector() {
        assert_eq!(scope_options(None), [DiffScope::All]);
        let clean = status(&[], false);
        assert_eq!(scope_options(Some(&clean)), [DiffScope::All]);
        let both = status(&["aaaa1", "bbbb2"], true);
        let labels: Vec<String> = scope_options(Some(&both))
            .iter()
            .map(DiffScope::label)
            .collect();
        assert_eq!(
            labels,
            [
                "All changes",
                "Committed changes",
                "Uncommitted changes",
                "aaa commit aaaa1",
                "bbb commit bbbb2"
            ]
        );
        let dirty_only = status(&[], true);
        assert!(should_reset_scope(&DiffScope::Committed, Some(&dirty_only)));
        assert!(!should_reset_scope(
            &DiffScope::Uncommitted,
            Some(&dirty_only)
        ));
        let gone = DiffScope::Commit {
            sha: "zzzz".into(),
            short_sha: "zzz".into(),
            subject: String::new(),
        };
        assert!(should_reset_scope(&gone, Some(&both)));
        assert_eq!(
            DiffScope::All.target(None),
            Some(WorkspaceDiffTarget::Uncommitted)
        );
        assert_eq!(DiffScope::Committed.target(None), None);
        assert_eq!(
            DiffScope::All.target(Some("main")),
            Some(WorkspaceDiffTarget::All {
                merge_base_branch: "main".into()
            })
        );
    }
}
