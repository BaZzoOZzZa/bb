use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

use bb_wire::plugin::{PluginCliContribution, PluginCliRequest, PluginCliResult};
use bb_wire::project::{ProjectKind, WorkspacePathListResponse};
use bb_wire::thread::{
    HighlightRange, ThreadListEntry, ThreadSearchMatch, ThreadSearchResponse,
    ThreadSearchSourceKind,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Color, Style};
use ratatui::text::Span;

use crate::app::{AppState, Effect, Focus, ToastLevel};
use crate::fuzzy::{match_paths, match_text};
use crate::input::{InputOutcome, LineInput};
use crate::keymap::Command;
use crate::settings::SettingsSection;
use crate::sidebar::{RowKey, project_display_label, project_label, project_labels, thread_title};
use crate::time::compact_relative_time;

pub const RESULT_LIMIT: usize = 50;
pub const RECENTS_LIMIT: usize = 8;
pub const RECENT_THREAD_LIMIT: usize = 20;
pub const THREAD_SEARCH_LIMIT_PER_GROUP: u32 = 20;
pub const THREAD_SEARCH_DEBOUNCE: Duration = Duration::from_millis(150);
pub const PATH_SEARCH_DEBOUNCE: Duration = Duration::from_millis(120);
pub const FILE_SEARCH_LIMIT: u32 = 50;
pub const MAX_PREVIEW_BYTES: usize = 512 * 1024;
const THREAD_SEARCH_MIN_CHARS: usize = 2;
const WINDOW_LEAD_CHARS: usize = 16;
const WINDOW_TAIL_CHARS: usize = 40;
const ELLIPSIS: char = '…';
const PAGE: isize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteKind {
    Commands,
    Threads,
    Projects,
    Files,
    PluginArgs,
}

impl PaletteKind {
    pub fn title(self) -> &'static str {
        match self {
            Self::Commands => "Commands",
            Self::Threads => "Threads",
            Self::Projects => "Projects",
            Self::Files => "Files",
            Self::PluginArgs => "Plugin command",
        }
    }

    pub fn placeholder(self) -> &'static str {
        match self {
            Self::Commands => "Type a command…",
            Self::Threads => "Search title, project, or message…",
            Self::Projects => "Switch to project…",
            Self::Files => "Search files…",
            Self::PluginArgs => "Arguments…",
        }
    }

    fn command(self) -> Option<Command> {
        match self {
            Self::Commands => Some(Command::PaletteOpen),
            Self::Threads => Some(Command::SearchThreads),
            Self::Projects => Some(Command::SwitchProject),
            Self::Files => Some(Command::QuickOpen),
            Self::PluginArgs => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginCommand {
    pub plugin_id: String,
    pub label: String,
    pub argv: Vec<String>,
    pub usage: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteTarget {
    Command(Command),
    Plugin(PluginCommand),
    Thread(String),
    Project(String),
    File(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Highlighted {
    pub text: String,
    pub positions: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteItem {
    pub id: String,
    pub group: String,
    pub title: String,
    pub positions: Vec<usize>,
    pub detail: Option<Highlighted>,
    pub hint: String,
    pub target: PaletteTarget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileSource {
    Environment {
        environment_id: String,
        root: Option<String>,
    },
    Project {
        project_id: String,
        root: Option<String>,
    },
}

impl FileSource {
    pub fn root(&self) -> Option<&str> {
        match self {
            Self::Environment { root, .. } | Self::Project { root, .. } => root.as_deref(),
        }
    }

    pub fn absolute(&self, path: &str) -> Option<String> {
        let root = self.root()?.trim_end_matches('/');
        Some(format!("{root}/{}", path.trim_start_matches('/')))
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Remote<T> {
    pub query: String,
    pub data: Option<T>,
    pub pending: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Palette {
    pub kind: PaletteKind,
    pub origin: PaletteKind,
    pub input: LineInput,
    pub selected: usize,
    pub offset: Cell<usize>,
    pub items: Vec<PaletteItem>,
    pub threads: Remote<ThreadSearchResponse>,
    pub paths: Remote<WorkspacePathListResponse>,
    pub source: Option<FileSource>,
    pub plugin: Option<PluginCommand>,
    pub contributions_loading: bool,
}

impl Palette {
    fn new(kind: PaletteKind) -> Self {
        Self {
            kind,
            origin: kind,
            input: LineInput::default(),
            selected: 0,
            offset: Cell::new(0),
            items: Vec::new(),
            threads: Remote::default(),
            paths: Remote::default(),
            source: None,
            plugin: None,
            contributions_loading: false,
        }
    }

    pub fn query(&self) -> &str {
        self.input.text()
    }

    pub fn selected_item(&self) -> Option<&PaletteItem> {
        self.items.get(self.selected)
    }

    fn move_selection(&mut self, delta: isize) {
        if self.items.is_empty() {
            self.selected = 0;
            return;
        }
        let count = self.items.len() as isize;
        let next = self.selected as isize + delta;
        self.selected = if delta.abs() == 1 {
            next.rem_euclid(count) as usize
        } else {
            next.clamp(0, count - 1) as usize
        };
    }

    pub fn notice(&self) -> Option<(String, bool)> {
        match self.kind {
            PaletteKind::Threads => {
                if let Some(error) = &self.threads.error {
                    return Some((format!("Search failed: {error}"), true));
                }
                if self.threads.pending.is_some() {
                    return Some(("Searching…".into(), false));
                }
                if self.items.is_empty() && !self.query().trim().is_empty() {
                    return Some(("No matching threads".into(), false));
                }
                None
            }
            PaletteKind::Files => {
                let Some(source) = &self.source else {
                    return Some((
                        "Open a thread or select a project to search its files".into(),
                        true,
                    ));
                };
                if let Some(error) = &self.paths.error {
                    return Some((format!("File search failed: {error}"), true));
                }
                if self.query().trim().is_empty() {
                    let place = source.root().unwrap_or("the workspace");
                    return Some((format!("Type to search files in {place}"), false));
                }
                if self.paths.pending.is_some() {
                    return Some(("Searching…".into(), false));
                }
                if self.items.is_empty() {
                    return Some(("No matching files".into(), false));
                }
                if self.paths.data.as_ref().is_some_and(|data| data.truncated) {
                    return Some(("More files match; refine the query".into(), false));
                }
                None
            }
            PaletteKind::PluginArgs => self.plugin.as_ref().map(|plugin| {
                let usage = if plugin.usage.is_empty() {
                    format!("bb {}", plugin.label)
                } else {
                    plugin.usage.clone()
                };
                (usage, false)
            }),
            PaletteKind::Commands if self.contributions_loading && self.items.is_empty() => {
                Some(("Loading plugin commands…".into(), false))
            }
            PaletteKind::Commands | PaletteKind::Projects if self.items.is_empty() => {
                Some(("No matches".into(), false))
            }
            PaletteKind::Commands | PaletteKind::Projects => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PaletteRequest {
    Contributions,
    SearchThreads {
        query: String,
    },
    Paths {
        source: FileSource,
        query: String,
    },
    File {
        source: FileSource,
        path: String,
    },
    RunCli {
        plugin_id: String,
        request: PluginCliRequest,
    },
}

impl PaletteRequest {
    pub fn debounce(&self) -> Duration {
        match self {
            Self::SearchThreads { .. } => THREAD_SEARCH_DEBOUNCE,
            Self::Paths { .. } => PATH_SEARCH_DEBOUNCE,
            _ => Duration::ZERO,
        }
    }

    pub fn slot(&self) -> Option<&'static str> {
        match self {
            Self::SearchThreads { .. } => Some("threads"),
            Self::Paths { .. } => Some("paths"),
            Self::File { .. } => Some("file"),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PaletteLoaded {
    Contributions(Result<Vec<PluginCliContribution>, String>),
    Threads {
        query: String,
        result: Result<ThreadSearchResponse, String>,
    },
    Paths {
        query: String,
        result: Result<WorkspacePathListResponse, String>,
    },
    File {
        path: String,
        result: Result<Vec<u8>, String>,
    },
    Cli {
        label: String,
        result: Result<PluginCliResult, String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum PaneBody {
    Loading,
    Error(String),
    Lines(Vec<Vec<Span<'static>>>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResultPane {
    pub generation: u64,
    pub title: String,
    pub body: PaneBody,
    pub scroll: usize,
    pub column: usize,
    pub viewport: Cell<usize>,
    pub open_target: Option<String>,
    pub copy_text: Option<String>,
    pub footer: Option<(String, Style)>,
}

impl ResultPane {
    fn new(generation: u64, title: String) -> Self {
        Self {
            generation,
            title,
            body: PaneBody::Loading,
            scroll: 0,
            column: 0,
            viewport: Cell::new(1),
            open_target: None,
            copy_text: None,
            footer: None,
        }
    }

    pub fn line_count(&self) -> usize {
        match &self.body {
            PaneBody::Lines(lines) => lines.len(),
            _ => 1,
        }
    }

    fn max_scroll(&self) -> usize {
        self.line_count().saturating_sub(self.viewport.get().max(1))
    }

    fn scroll_by(&mut self, delta: isize) {
        self.scroll = self
            .scroll
            .saturating_add_signed(delta)
            .min(self.max_scroll());
    }
}

struct CommandEntry {
    command: Command,
    group: &'static str,
    title: &'static str,
    needs_thread: bool,
}

const fn entry(
    command: Command,
    group: &'static str,
    title: &'static str,
    needs_thread: bool,
) -> CommandEntry {
    CommandEntry {
        command,
        group,
        title,
        needs_thread,
    }
}

const THREADS: &str = "Threads";
const LAYOUT: &str = "Window and layout";
const WORKSPACE: &str = "Workspace";
const COMPOSER: &str = "Composer and models";
const TIMELINE: &str = "Timeline";
const SETTINGS: &str = "Settings";
const AUTOMATIONS: &str = "Automations";

const COMMAND_ENTRIES: &[CommandEntry] = &[
    entry(Command::NewThread, THREADS, "New thread", false),
    entry(Command::SearchThreads, THREADS, "Search threads", false),
    entry(Command::Rename, THREADS, "Rename thread", true),
    entry(Command::Archive, THREADS, "Archive thread", true),
    entry(Command::ThreadPrevious, THREADS, "Previous thread", false),
    entry(Command::ThreadNext, THREADS, "Next thread", false),
    entry(
        Command::NewThreadInEnvironment,
        THREADS,
        "New thread in this environment",
        true,
    ),
    entry(Command::SwitchProject, THREADS, "Switch project", false),
    entry(Command::Filter, THREADS, "Filter sidebar threads", false),
    entry(
        Command::ToggleRead,
        THREADS,
        "Mark thread read / unread",
        true,
    ),
    entry(Command::TogglePin, THREADS, "Pin / unpin thread", true),
    entry(Command::UndoArchive, THREADS, "Undo last archive", false),
    entry(
        Command::NextPending,
        THREADS,
        "Answer the next waiting request",
        false,
    ),
    entry(Command::Notifications, LAYOUT, "Show notifications", false),
    entry(Command::Reload, LAYOUT, "Reload data", false),
    entry(Command::SidebarToggle, LAYOUT, "Toggle sidebar", false),
    entry(Command::PanelToggle, LAYOUT, "Toggle panel", false),
    entry(Command::FocusPrevious, LAYOUT, "Focus previous pane", false),
    entry(Command::FocusNext, LAYOUT, "Focus next pane", false),
    entry(Command::Help, LAYOUT, "Show keyboard shortcuts", false),
    entry(Command::Quit, LAYOUT, "Quit", false),
    entry(Command::QuickOpen, WORKSPACE, "Quick open file", false),
    entry(Command::DiffToggle, WORKSPACE, "Toggle git diff", true),
    entry(Command::PanelNextTab, WORKSPACE, "Next panel tab", false),
    entry(
        Command::PanelPreviousTab,
        WORKSPACE,
        "Previous panel tab",
        false,
    ),
    entry(
        Command::PanelFullScreen,
        WORKSPACE,
        "Maximize / restore panel",
        false,
    ),
    entry(Command::TerminalOpen, WORKSPACE, "Open terminal", true),
    entry(Command::TerminalNew, WORKSPACE, "New terminal", true),
    entry(
        Command::TerminalAttach,
        WORKSPACE,
        "Attach terminal fullscreen",
        true,
    ),
    entry(Command::GitCommit, WORKSPACE, "Commit changes", true),
    entry(
        Command::PullRequestMerge,
        WORKSPACE,
        "Merge pull request",
        true,
    ),
    entry(
        Command::PullRequestReady,
        WORKSPACE,
        "Mark pull request ready for review",
        true,
    ),
    entry(
        Command::PullRequestDraft,
        WORKSPACE,
        "Convert pull request to draft",
        true,
    ),
    entry(Command::FocusComposer, COMPOSER, "Focus composer", true),
    entry(Command::ModelPicker, COMPOSER, "Pick model", true),
    entry(
        Command::ReasoningPicker,
        COMPOSER,
        "Pick reasoning effort",
        true,
    ),
    entry(
        Command::PermissionPicker,
        COMPOSER,
        "Pick permission mode",
        true,
    ),
    entry(Command::StopTurn, COMPOSER, "Stop the running turn", true),
    entry(Command::QueueList, COMPOSER, "Queued messages", true),
    entry(Command::AttachFile, COMPOSER, "Attach file", false),
    entry(Command::Attachments, COMPOSER, "Remove attachment", false),
    entry(
        Command::ToggleSubmitMode,
        COMPOSER,
        "Toggle Enter while running: queue / steer",
        false,
    ),
    entry(
        Command::SplitOpen,
        LAYOUT,
        "Open thread in split pane",
        false,
    ),
    entry(Command::PaneLeft, LAYOUT, "Focus chat pane left", false),
    entry(Command::PaneRight, LAYOUT, "Focus chat pane right", false),
    entry(Command::PaneClose, LAYOUT, "Close focused chat pane", false),
    entry(
        Command::PaneMaximize,
        LAYOUT,
        "Toggle focused chat pane size",
        false,
    ),
    entry(Command::Automations, AUTOMATIONS, "Open automations", false),
    entry(Command::NewAutomation, AUTOMATIONS, "New automation", false),
    entry(Command::Settings, SETTINGS, "Open settings", false),
    entry(
        Command::SettingsSection(SettingsSection::Providers),
        SETTINGS,
        "Provider settings",
        false,
    ),
    entry(
        Command::SettingsSection(SettingsSection::Projects),
        SETTINGS,
        "Project settings",
        false,
    ),
    entry(
        Command::SettingsSection(SettingsSection::Machines),
        SETTINGS,
        "Machine settings",
        false,
    ),
    entry(
        Command::SettingsSection(SettingsSection::Plugins),
        SETTINGS,
        "Installed plugins",
        false,
    ),
    entry(
        Command::SettingsSection(SettingsSection::Archived),
        SETTINGS,
        "Archived threads",
        false,
    ),
    entry(
        Command::SettingsSection(SettingsSection::Keyboard),
        SETTINGS,
        "Keyboard shortcuts settings",
        false,
    ),
    entry(Command::Copy, TIMELINE, "Copy selected row", true),
    entry(
        Command::OpenExternal,
        TIMELINE,
        "Open link or file of selected row",
        true,
    ),
];

pub fn palette_command_ids() -> Vec<Command> {
    COMMAND_ENTRIES.iter().map(|entry| entry.command).collect()
}

pub fn action_id(command: Command) -> String {
    match command.app_command_id() {
        Some(id) => format!("app:{id}"),
        None => format!("tui:{command:?}"),
    }
}

pub fn plugin_argv(contribution_name: &str, command_name: &str, usage: &str) -> Vec<String> {
    let mut words = usage.split_whitespace().peekable();
    if words.peek() == Some(&"bb") {
        words.next();
    }
    if words.peek() == Some(&contribution_name) {
        words.next();
        let argv: Vec<String> = words
            .take_while(|word| {
                !word.starts_with(['<', '[', '-', '(', '{', '|']) && !word.contains(['<', '|'])
            })
            .map(str::to_owned)
            .collect();
        if !argv.is_empty() {
            return argv;
        }
    }
    if command_name.is_empty() {
        Vec::new()
    } else {
        vec![command_name.to_owned()]
    }
}

pub fn plugin_commands(contributions: &[PluginCliContribution]) -> Vec<PluginCommand> {
    let mut commands = Vec::new();
    for contribution in contributions {
        if contribution.commands.is_empty() {
            commands.push(PluginCommand {
                plugin_id: contribution.plugin_id.clone(),
                label: contribution.name.clone(),
                argv: Vec::new(),
                usage: String::new(),
                summary: contribution.summary.clone(),
            });
            continue;
        }
        for command in &contribution.commands {
            let argv = plugin_argv(&contribution.name, &command.name, &command.usage);
            let label = std::iter::once(contribution.name.as_str())
                .chain(argv.iter().map(String::as_str))
                .collect::<Vec<_>>()
                .join(" ");
            commands.push(PluginCommand {
                plugin_id: contribution.plugin_id.clone(),
                label,
                argv,
                usage: command.usage.clone(),
                summary: if command.summary.is_empty() {
                    contribution.summary.clone()
                } else {
                    command.summary.clone()
                },
            });
        }
    }
    commands
}

pub fn split_argv(text: &str) -> Result<Vec<String>, String> {
    let mut argv = Vec::new();
    let mut current = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        match (quote, ch) {
            (Some(open), ch) if ch == open => quote = None,
            (Some('"'), '\\') => match chars.next() {
                Some(next) => current.push(next),
                None => return Err("Unfinished escape".into()),
            },
            (Some(_), ch) => current.push(ch),
            (None, '"' | '\'') => {
                quote = Some(ch);
                in_word = true;
            }
            (None, '\\') => match chars.next() {
                Some(next) => {
                    current.push(next);
                    in_word = true;
                }
                None => return Err("Unfinished escape".into()),
            },
            (None, ch) if ch.is_whitespace() => {
                if in_word {
                    argv.push(std::mem::take(&mut current));
                    in_word = false;
                }
            }
            (None, ch) => {
                current.push(ch);
                in_word = true;
            }
        }
    }
    if quote.is_some() {
        return Err("Unclosed quote".into());
    }
    if in_word {
        argv.push(current);
    }
    Ok(argv)
}

pub fn title_match_positions(title: &str, query: &str) -> Vec<usize> {
    let haystack: Vec<char> = title
        .chars()
        .map(|ch| ch.to_lowercase().next().unwrap_or(ch))
        .collect();
    let mut positions = Vec::new();
    let mut cursor = 0;
    for ch in query.chars() {
        if ch == ' ' {
            continue;
        }
        let ch = ch.to_lowercase().next().unwrap_or(ch);
        match haystack[cursor.min(haystack.len())..]
            .iter()
            .position(|candidate| *candidate == ch)
        {
            Some(found) => {
                positions.push(cursor + found);
                cursor += found + 1;
            }
            None => return Vec::new(),
        }
    }
    positions
}

pub fn rank_actions(items: Vec<PaletteItem>, query: &str, recents: &[String]) -> Vec<PaletteItem> {
    if query.trim().is_empty() {
        let recent_rank: HashMap<&str, usize> = recents
            .iter()
            .enumerate()
            .map(|(index, id)| (id.as_str(), index))
            .collect();
        let (mut recent, rest): (Vec<PaletteItem>, Vec<PaletteItem>) = items
            .into_iter()
            .partition(|item| recent_rank.contains_key(item.id.as_str()));
        recent.sort_by_key(|item| recent_rank[item.id.as_str()]);
        for item in &mut recent {
            item.group = "Recent".into();
        }
        recent.extend(rest);
        return recent;
    }
    let matches = match_text(
        &items,
        query,
        |item| item.title.as_str(),
        |item| vec![item.group.as_str()],
        RESULT_LIMIT,
    );
    let recent_rank = |item: &PaletteItem| {
        recents
            .iter()
            .position(|id| *id == item.id)
            .unwrap_or(usize::MAX)
    };
    let mut ranked: Vec<(i64, usize, usize)> = matches
        .iter()
        .map(|found| (found.score, recent_rank(&items[found.index]), found.index))
        .collect();
    ranked.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then(left.1.cmp(&right.1))
            .then(left.2.cmp(&right.2))
    });
    let mut slots: Vec<Option<PaletteItem>> = items.into_iter().map(Some).collect();
    ranked
        .into_iter()
        .filter_map(|(_, _, index)| slots[index].take())
        .map(|mut item| {
            item.positions = title_match_positions(&item.title, query);
            item
        })
        .collect()
}

pub fn record_recent(recents: &mut Vec<String>, id: &str) {
    recents.retain(|entry| entry != id);
    recents.insert(0, id.to_owned());
    recents.truncate(RECENTS_LIMIT);
}

pub fn utf16_ranges_to_char_ranges(text: &str, ranges: &[HighlightRange]) -> Vec<(usize, usize)> {
    let mut boundaries = Vec::with_capacity(text.len() + 1);
    let mut units = 0usize;
    for ch in text.chars() {
        boundaries.push(units);
        units += ch.len_utf16();
    }
    boundaries.push(units);
    let to_char = |unit: usize, round_up: bool| match boundaries.binary_search(&unit) {
        Ok(index) => index,
        Err(index) if round_up => index.min(boundaries.len() - 1),
        Err(index) => index.saturating_sub(1),
    };
    let mut out: Vec<(usize, usize)> = ranges
        .iter()
        .filter_map(|range| {
            let start = (range.start as usize).min(units);
            let end = (range.end as usize).min(units);
            if end <= start {
                return None;
            }
            Some((to_char(start, false), to_char(end, true)))
        })
        .filter(|(start, end)| end > start)
        .collect();
    out.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (start, end) in out {
        match merged.last_mut() {
            Some(last) if start < last.1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    merged
}

fn positions_of(ranges: &[(usize, usize)]) -> Vec<usize> {
    ranges
        .iter()
        .flat_map(|(start, end)| *start..*end)
        .collect()
}

fn positions_to_ranges(positions: &[usize]) -> Vec<(usize, usize)> {
    let mut sorted = positions.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for position in sorted {
        match ranges.last_mut() {
            Some(last) if last.1 == position => last.1 = position + 1,
            _ => ranges.push((position, position + 1)),
        }
    }
    ranges
}

pub fn window_excerpt(text: &str, ranges: &[(usize, usize)]) -> Highlighted {
    let chars: Vec<char> = text.chars().collect();
    let Some(first) = ranges.first().copied() else {
        return Highlighted {
            text: text.to_owned(),
            positions: Vec::new(),
        };
    };
    let len = chars.len();
    let mut start = first.0.saturating_sub(WINDOW_LEAD_CHARS);
    let mut end = (first.1 + WINDOW_TAIL_CHARS).min(len);
    if start > 0
        && let Some(offset) = chars[start..first.0]
            .iter()
            .position(|ch| ch.is_whitespace())
    {
        start += offset + 1;
        while start < first.0 && chars[start].is_whitespace() {
            start += 1;
        }
    }
    if end < len {
        let tail = &chars[first.1..end];
        let mut last_space = None;
        for (index, ch) in tail.iter().enumerate() {
            if ch.is_whitespace() && tail[index + 1..].iter().all(|ch| !ch.is_whitespace()) {
                last_space = Some(index);
                break;
            }
        }
        if let Some(index) = last_space.filter(|index| *index > 0) {
            end = first.1 + index;
        }
    }
    let prefix = usize::from(start > 0);
    let mut out = String::new();
    if start > 0 {
        out.push(ELLIPSIS);
    }
    out.extend(&chars[start..end]);
    if end < len {
        out.push(ELLIPSIS);
    }
    let positions = ranges
        .iter()
        .flat_map(|(range_start, range_end)| {
            let from = (*range_start).max(start);
            let to = (*range_end).min(end);
            (from..to.max(from)).map(move |position| position - start + prefix)
        })
        .collect();
    Highlighted {
        text: out,
        positions,
    }
}

fn search_words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        let ch = nucleo_matcher::chars::normalize(ch);
        if ch.is_alphanumeric() {
            current.extend(ch.to_lowercase());
        } else if !current.is_empty() {
            words.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

fn non_whitespace_chars(text: &str) -> usize {
    text.chars().filter(|ch| !ch.is_whitespace()).count()
}

pub fn is_searchable(query: &str) -> bool {
    non_whitespace_chars(query) >= THREAD_SEARCH_MIN_CHARS
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadRowData {
    pub archived: bool,
    pub thread_id: String,
    pub title: String,
    pub title_ranges: Vec<(usize, usize)>,
    pub excerpt: Option<Highlighted>,
    pub project_name: Option<String>,
    pub project_ranges: Vec<(usize, usize)>,
    pub updated_at: i64,
}

fn is_title_match(found: &ThreadSearchMatch) -> bool {
    matches!(
        found.source_kind,
        ThreadSearchSourceKind::Title | ThreadSearchSourceKind::TitleFallback
    )
}

pub struct ThreadSearchInput<'a> {
    pub threads: &'a [ThreadListEntry],
    pub project_names: &'a HashMap<String, String>,
    pub personal_ids: &'a HashSet<String>,
    pub query: &'a str,
    pub response: Option<&'a ThreadSearchResponse>,
}

fn project_name(input: &ThreadSearchInput<'_>, project_id: &str) -> Option<String> {
    if input.personal_ids.contains(project_id) {
        return None;
    }
    input.project_names.get(project_id).cloned()
}

fn server_row(
    input: &ThreadSearchInput<'_>,
    entry: &ThreadListEntry,
    matches: &[ThreadSearchMatch],
    archived: bool,
) -> ThreadRowData {
    let title = thread_title(entry);
    let title_ranges = matches
        .iter()
        .find(|found| is_title_match(found) && found.text == title)
        .map(|found| utf16_ranges_to_char_ranges(&found.text, &found.highlight_ranges))
        .unwrap_or_default();
    let excerpt = matches
        .iter()
        .find(|found| !is_title_match(found))
        .map(|found| {
            let ranges = utf16_ranges_to_char_ranges(&found.text, &found.highlight_ranges);
            window_excerpt(&found.text, &ranges)
        });
    ThreadRowData {
        archived,
        thread_id: entry.thread.id.clone(),
        title,
        title_ranges,
        excerpt,
        project_name: project_name(input, &entry.thread.project_id),
        project_ranges: Vec::new(),
        updated_at: entry.thread.updated_at,
    }
}

fn project_name_match(name: &str, query: &str) -> Option<(usize, usize)> {
    let needle: Vec<char> = query.to_lowercase().chars().collect();
    let haystack: Vec<char> = name.to_lowercase().chars().collect();
    if haystack.starts_with(&needle) {
        return Some((0, needle.len()));
    }
    let mut index = 0;
    while index < haystack.len() {
        if haystack[index].is_alphanumeric() {
            let start = index;
            while index < haystack.len() && haystack[index].is_alphanumeric() {
                index += 1;
            }
            if haystack[start..index].starts_with(&needle) {
                return Some((start, start + needle.len()));
            }
        } else {
            index += 1;
        }
    }
    None
}

fn local_rows(
    input: &ThreadSearchInput<'_>,
    active: &[&ThreadListEntry],
    query: &str,
) -> Vec<ThreadRowData> {
    let titles: Vec<String> = active.iter().map(|entry| thread_title(entry)).collect();
    let mut matches = match_text(&titles, query, String::as_str, |_| Vec::new(), titles.len());
    matches.sort_by(|left, right| {
        right.score.cmp(&left.score).then(
            active[right.index]
                .thread
                .updated_at
                .cmp(&active[left.index].thread.updated_at),
        )
    });
    let title_ids: HashSet<&str> = matches
        .iter()
        .map(|found| active[found.index].thread.id.as_str())
        .collect();
    let mut rows: Vec<ThreadRowData> = matches
        .iter()
        .take(RESULT_LIMIT)
        .map(|found| {
            let mut row = server_row(input, active[found.index], &[], false);
            row.title_ranges = positions_to_ranges(&found.positions);
            row
        })
        .collect();
    let remaining = RESULT_LIMIT.saturating_sub(rows.len());
    if remaining == 0 {
        return rows;
    }
    let mut project_matches: HashMap<&str, (usize, usize)> = HashMap::new();
    for (project_id, name) in input.project_names {
        if input.personal_ids.contains(project_id) {
            continue;
        }
        if let Some(found) = project_name_match(name, query) {
            project_matches.insert(project_id.as_str(), found);
        }
    }
    if project_matches.is_empty() {
        return rows;
    }
    let mut by_project: Vec<(&ThreadListEntry, (usize, usize))> = active
        .iter()
        .filter(|entry| !title_ids.contains(entry.thread.id.as_str()))
        .filter_map(|entry| {
            project_matches
                .get(entry.thread.project_id.as_str())
                .map(|found| (*entry, *found))
        })
        .collect();
    by_project.sort_by_key(|(entry, _)| std::cmp::Reverse(entry.thread.updated_at));
    rows.extend(
        by_project
            .into_iter()
            .take(remaining)
            .map(|(entry, found)| {
                let mut row = server_row(input, entry, &[], false);
                row.project_ranges = vec![found];
                row
            }),
    );
    rows
}

fn title_matches_first(rows: Vec<ThreadRowData>, query: &str) -> Vec<ThreadRowData> {
    let query_words = search_words(query);
    let (titles, others): (Vec<ThreadRowData>, Vec<ThreadRowData>) =
        rows.into_iter().partition(|row| {
            let title_words = search_words(&row.title);
            !query_words.is_empty()
                && query_words.iter().all(|query_word| {
                    title_words
                        .iter()
                        .any(|title_word| title_word.starts_with(query_word.as_str()))
                })
        });
    titles.into_iter().chain(others).collect()
}

pub fn build_thread_rows(input: &ThreadSearchInput<'_>) -> (bool, Vec<ThreadRowData>) {
    let query = input.query.trim();
    let active: Vec<&ThreadListEntry> = input
        .threads
        .iter()
        .filter(|entry| entry.thread.archived_at.is_none())
        .collect();
    if query.is_empty() {
        let mut recent = active;
        recent.sort_by_key(|entry| std::cmp::Reverse(entry.thread.updated_at));
        let rows = recent
            .into_iter()
            .take(RECENT_THREAD_LIMIT)
            .map(|entry| server_row(input, entry, &[], false))
            .collect();
        return (true, rows);
    }
    let response = input.response.filter(|_| is_searchable(query));
    let server_active: Vec<ThreadRowData> = response
        .map(|response| {
            response
                .active
                .results
                .iter()
                .map(|result| server_row(input, &result.thread, &result.matches, false))
                .collect()
        })
        .unwrap_or_default();
    let server_archived: Vec<ThreadRowData> = response
        .map(|response| {
            response
                .archived
                .results
                .iter()
                .map(|result| server_row(input, &result.thread, &result.matches, true))
                .collect()
        })
        .unwrap_or_default();
    let local = local_rows(input, &active, query);
    let local_ids: HashSet<String> = local.iter().map(|row| row.thread_id.clone()).collect();
    let server_by_id: HashMap<&str, &ThreadRowData> = server_active
        .iter()
        .map(|row| (row.thread_id.as_str(), row))
        .collect();
    let merged: Vec<ThreadRowData> = local
        .into_iter()
        .map(|mut row| {
            if let Some(server) = server_by_id.get(row.thread_id.as_str()) {
                if row.title_ranges.is_empty() {
                    row.title_ranges = server.title_ranges.clone();
                }
                row.excerpt = server.excerpt.clone();
            }
            row
        })
        .collect();
    let extra: Vec<ThreadRowData> = server_active
        .iter()
        .filter(|row| !local_ids.contains(&row.thread_id))
        .cloned()
        .collect();
    let mut rows = title_matches_first(merged.into_iter().chain(extra).collect(), query);
    rows.extend(title_matches_first(server_archived, query));
    (false, rows)
}

fn thread_items(rows: Vec<ThreadRowData>, recent: bool, now_ms: i64) -> Vec<PaletteItem> {
    rows.into_iter()
        .map(|row| {
            let detail = match (row.excerpt, row.project_name) {
                (Some(excerpt), _) => Some(excerpt),
                (None, Some(name)) => Some(Highlighted {
                    positions: positions_of(&row.project_ranges),
                    text: name,
                }),
                (None, None) => None,
            };
            let group = if row.archived {
                "Archived"
            } else if recent {
                "Recent threads"
            } else {
                "Threads"
            };
            PaletteItem {
                id: format!(
                    "{}:{}",
                    if row.archived { "archived" } else { "active" },
                    row.thread_id
                ),
                group: group.into(),
                positions: positions_of(&row.title_ranges),
                title: row.title,
                detail,
                hint: compact_relative_time(row.updated_at, now_ms),
                target: PaletteTarget::Thread(row.thread_id),
            }
        })
        .collect()
}

pub fn pane_lines_for_cli(result: &PluginCliResult) -> Vec<Vec<Span<'static>>> {
    let mut lines = Vec::new();
    if !result.stdout.is_empty() {
        lines.extend(crate::ansi::ansi_lines(
            result.stdout.trim_end_matches('\n'),
            Style::new(),
        ));
    }
    if !result.stderr.is_empty() {
        lines.extend(crate::ansi::ansi_lines(
            result.stderr.trim_end_matches('\n'),
            Style::new().fg(Color::Yellow),
        ));
    }
    if let Some(error) = &result.error {
        let text = match error {
            serde_json::Value::String(text) => text.clone(),
            other => other.to_string(),
        };
        lines.push(vec![Span::styled(text, Style::new().fg(Color::Red))]);
    }
    if lines.is_empty() {
        lines.push(vec![Span::styled(
            "(no output)",
            Style::new().fg(Color::DarkGray),
        )]);
    }
    lines
}

pub fn extension(path: &str) -> Option<&str> {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => Some(ext),
        _ => match name {
            "Dockerfile" => Some("dockerfile"),
            "Makefile" => Some("makefile"),
            _ => None,
        },
    }
}

pub fn pane_lines_for_file(path: &str, bytes: &[u8]) -> (Vec<Vec<Span<'static>>>, Option<String>) {
    let probe = &bytes[..bytes.len().min(8192)];
    if probe.contains(&0) {
        return (
            vec![vec![Span::styled(
                format!("Binary file ({} bytes)", bytes.len()),
                Style::new().fg(Color::DarkGray),
            )]],
            None,
        );
    }
    let truncated = bytes.len() > MAX_PREVIEW_BYTES;
    let shown = &bytes[..bytes.len().min(MAX_PREVIEW_BYTES)];
    let text = String::from_utf8_lossy(shown);
    let lines = crate::highlight::highlight_code(&text, extension(path), Style::new());
    let note = truncated.then(|| {
        format!(
            "Showing the first {} KiB of {} KiB",
            MAX_PREVIEW_BYTES / 1024,
            bytes.len() / 1024
        )
    });
    (lines, note)
}

impl AppState {
    pub fn palette_target_thread(&self) -> Option<String> {
        self.open_thread
            .clone()
            .or_else(|| self.selected_thread_id().map(str::to_owned))
    }

    fn file_source(&self) -> Option<FileSource> {
        if let Some(thread_id) = self.palette_target_thread() {
            let detail = self.snapshot.thread(&thread_id);
            let entry = self.sidebar_entry(&thread_id);
            let environment_id = detail
                .and_then(|detail| detail.thread.environment_id.clone())
                .or_else(|| entry.and_then(|entry| entry.thread.environment_id.clone()))
                .or_else(|| {
                    detail
                        .and_then(|detail| detail.environment.as_ref())
                        .map(|environment| environment.id.clone())
                })
                .filter(|id| !id.is_empty());
            let root = detail
                .and_then(|detail| detail.environment.as_ref())
                .and_then(|environment| environment.path.clone());
            if let Some(environment_id) = environment_id {
                return Some(FileSource::Environment {
                    environment_id,
                    root,
                });
            }
            let project_id = detail
                .map(|detail| detail.thread.project_id.clone())
                .or_else(|| entry.map(|entry| entry.thread.project_id.clone()));
            if let Some(project_id) = project_id
                && !self.is_personal_project(&project_id)
            {
                return Some(FileSource::Project { project_id, root });
            }
            return None;
        }
        match self.selected_row() {
            Some(crate::sidebar::SidebarRow::Group(group))
                if self.project_name(&group.key).is_some()
                    && !self.is_personal_project(&group.key) =>
            {
                Some(FileSource::Project {
                    project_id: group.key.clone(),
                    root: None,
                })
            }
            _ => None,
        }
    }

    fn is_personal_project(&self, project_id: &str) -> bool {
        self.snapshot
            .sidebar
            .data
            .as_deref()
            .is_some_and(|sidebar| {
                sidebar.personal_project.project.id == project_id
                    || sidebar.projects.iter().any(|project| {
                        project.project.id == project_id
                            && project.project.kind == ProjectKind::Personal
                    })
            })
    }

    pub(crate) fn project_name(&self, project_id: &str) -> Option<String> {
        let sidebar = self.snapshot.sidebar.data.as_deref()?;
        project_display_label(sidebar, project_id)
    }

    fn next_palette_generation(&mut self) -> u64 {
        self.palette_generation += 1;
        self.palette_generation
    }

    pub fn open_palette(&mut self, kind: PaletteKind) -> Vec<Effect> {
        let generation = self.next_palette_generation();
        let mut palette = Palette::new(kind);
        let mut effects = Vec::new();
        if kind == PaletteKind::Files {
            palette.source = self.file_source();
        }
        if kind == PaletteKind::Commands {
            palette.contributions_loading = self.plugin_contributions.is_none();
            effects.push(Effect::Palette {
                generation,
                request: PaletteRequest::Contributions,
            });
        }
        self.palette = Some(palette);
        self.refresh_palette_items();
        effects
    }

    fn switch_palette(&mut self, kind: PaletteKind) -> Vec<Effect> {
        let origin = self.palette.as_ref().map_or(kind, |palette| palette.origin);
        let effects = self.open_palette(kind);
        if let Some(palette) = self.palette.as_mut() {
            palette.origin = origin;
        }
        effects
    }

    pub fn close_palette(&mut self) {
        if let Some(palette) = self.palette.take()
            && palette.kind != PaletteKind::PluginArgs
        {
            self.last_palette = Some((palette.kind, palette.query().to_owned(), palette.selected));
        }
        self.palette_generation += 1;
    }

    pub(crate) fn open_palette_with(&mut self, kind: PaletteKind, query: &str) -> Vec<Effect> {
        let mut effects = self.open_palette(kind);
        if query.is_empty() {
            return effects;
        }
        if let Some(palette) = self.palette.as_mut() {
            palette.input = LineInput::new(query);
        }
        effects.extend(self.palette_query_changed());
        effects
    }

    pub(crate) fn resume_palette(&mut self) -> Vec<Effect> {
        let Some((kind, query, selected)) = self.last_palette.clone() else {
            self.keys.error("No minibuffer to resume");
            return Vec::new();
        };
        let effects = self.open_palette_with(kind, &query);
        if let Some(palette) = self.palette.as_mut() {
            palette.selected = selected.min(palette.items.len().saturating_sub(1));
        }
        effects
    }

    fn command_items(&self) -> Vec<PaletteItem> {
        let has_thread = self.open_thread.is_some();
        let has_target = self.palette_target_thread().is_some();
        let mut items: Vec<PaletteItem> = COMMAND_ENTRIES
            .iter()
            .filter(|entry| {
                !entry.needs_thread
                    || match entry.command {
                        Command::Rename
                        | Command::Archive
                        | Command::ToggleRead
                        | Command::TogglePin => has_target,
                        _ => has_thread,
                    }
            })
            .map(|entry| PaletteItem {
                id: action_id(entry.command),
                group: entry.group.into(),
                title: entry.title.into(),
                positions: Vec::new(),
                detail: None,
                hint: self.keymap.label_for(entry.command),
                target: PaletteTarget::Command(entry.command),
            })
            .collect();
        if let Some(contributions) = &self.plugin_contributions {
            items.extend(
                plugin_commands(contributions)
                    .into_iter()
                    .map(|command| PaletteItem {
                        id: format!("plugin-cli:{}/{}", command.plugin_id, command.label),
                        group: "Plugin commands".into(),
                        title: format!("bb {}", command.label),
                        positions: Vec::new(),
                        detail: (!command.summary.is_empty()).then(|| Highlighted {
                            text: command.summary.clone(),
                            positions: Vec::new(),
                        }),
                        hint: String::new(),
                        target: PaletteTarget::Plugin(command),
                    }),
            );
        }
        items
    }

    fn project_items(&self, query: &str) -> Vec<PaletteItem> {
        let Some(sidebar) = self.snapshot.sidebar.data.as_deref() else {
            return Vec::new();
        };
        let projects: Vec<_> = sidebar
            .projects
            .iter()
            .chain(std::iter::once(&sidebar.personal_project))
            .filter(|project| !project.project.id.is_empty())
            .collect();
        let names = project_labels(sidebar);
        let labels: Vec<String> = projects
            .iter()
            .map(|project| {
                names
                    .get(&project.project.id)
                    .cloned()
                    .unwrap_or_else(|| project_label(project))
            })
            .collect();
        match_text(
            &labels,
            query,
            String::as_str,
            |_| Vec::new(),
            RESULT_LIMIT.max(labels.len()),
        )
        .into_iter()
        .map(|found| {
            let project = projects[found.index];
            let count = project
                .threads
                .iter()
                .filter(|entry| !entry.thread.is_archived())
                .count();
            PaletteItem {
                id: format!("project:{}", project.project.id),
                group: "Projects".into(),
                title: labels[found.index].clone(),
                positions: found.positions,
                detail: None,
                hint: if count == 1 {
                    "1 thread".to_owned()
                } else {
                    format!("{count} threads")
                },
                target: PaletteTarget::Project(project.project.id.clone()),
            }
        })
        .collect()
    }

    fn thread_palette_items(&self, palette: &Palette) -> Vec<PaletteItem> {
        let Some(sidebar) = self.snapshot.sidebar.data.as_deref() else {
            return Vec::new();
        };
        let mut project_names = project_labels(sidebar);
        let mut personal_ids = HashSet::new();
        personal_ids.insert(sidebar.personal_project.project.id.clone());
        for project in sidebar
            .projects
            .iter()
            .chain(std::iter::once(&sidebar.personal_project))
        {
            if project.project.kind == ProjectKind::Personal {
                personal_ids.insert(project.project.id.clone());
            }
            project_names
                .entry(project.project.id.clone())
                .or_insert_with(|| project_label(project));
        }
        let threads: Vec<ThreadListEntry> = sidebar
            .projects
            .iter()
            .chain(std::iter::once(&sidebar.personal_project))
            .flat_map(|project| project.threads.iter().cloned())
            .collect();
        let trimmed = palette.query().trim();
        let response = palette
            .threads
            .data
            .as_ref()
            .filter(|_| palette.threads.query == trimmed);
        let input = ThreadSearchInput {
            threads: &threads,
            project_names: &project_names,
            personal_ids: &personal_ids,
            query: palette.query(),
            response,
        };
        let (recent, rows) = build_thread_rows(&input);
        thread_items(rows, recent, self.now_ms)
    }

    fn file_items(palette: &Palette) -> Vec<PaletteItem> {
        let query = palette.query().trim();
        if query.is_empty() {
            return Vec::new();
        }
        let Some(data) = &palette.paths.data else {
            return Vec::new();
        };
        let entries: Vec<_> = data.paths.iter().collect();
        let fresh = palette.paths.query == query;
        let ordered: Vec<(usize, Vec<usize>)> = if fresh {
            let mut indexed: Vec<usize> = (0..entries.len()).collect();
            indexed.sort_by(|left, right| {
                entries[*right]
                    .score
                    .partial_cmp(&entries[*left].score)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(left.cmp(right))
            });
            indexed
                .into_iter()
                .map(|index| {
                    let ranges: Vec<HighlightRange> = entries[index]
                        .positions
                        .iter()
                        .map(|position| HighlightRange {
                            start: *position,
                            end: position + 1,
                        })
                        .collect();
                    (
                        index,
                        positions_of(&utf16_ranges_to_char_ranges(&entries[index].path, &ranges)),
                    )
                })
                .collect()
        } else {
            match_paths(&entries, query, |entry| entry.path.as_str(), RESULT_LIMIT)
                .into_iter()
                .map(|found| (found.index, found.positions))
                .collect()
        };
        ordered
            .into_iter()
            .take(FILE_SEARCH_LIMIT as usize)
            .map(|(index, positions)| {
                let entry = entries[index];
                PaletteItem {
                    id: format!("file:{}", entry.path),
                    group: "Files".into(),
                    title: entry.path.clone(),
                    positions,
                    detail: None,
                    hint: String::new(),
                    target: PaletteTarget::File(entry.path.clone()),
                }
            })
            .collect()
    }

    pub fn refresh_palette_items(&mut self) {
        let Some(palette) = self.palette.as_ref() else {
            return;
        };
        let previous = palette.selected_item().map(|item| item.id.clone());
        let items = match palette.kind {
            PaletteKind::Commands => {
                rank_actions(self.command_items(), palette.query(), &self.palette_recents)
            }
            PaletteKind::Threads => self.thread_palette_items(palette),
            PaletteKind::Projects => self.project_items(palette.query()),
            PaletteKind::Files => Self::file_items(palette),
            PaletteKind::PluginArgs => Vec::new(),
        };
        let Some(palette) = self.palette.as_mut() else {
            return;
        };
        palette.selected = previous
            .and_then(|id| items.iter().position(|item| item.id == id))
            .unwrap_or(0);
        palette.items = items;
    }

    fn palette_query_changed(&mut self) -> Vec<Effect> {
        let generation = self.palette_generation;
        let Some(palette) = self.palette.as_mut() else {
            return Vec::new();
        };
        palette.selected = 0;
        let query = palette.query().trim().to_owned();
        let mut effects = Vec::new();
        match palette.kind {
            PaletteKind::Threads => {
                palette.threads.error = None;
                if is_searchable(&query) {
                    if palette.threads.query != query || palette.threads.data.is_none() {
                        palette.threads.pending = Some(query.clone());
                        effects.push(Effect::Palette {
                            generation,
                            request: PaletteRequest::SearchThreads { query },
                        });
                    } else {
                        palette.threads.pending = None;
                    }
                } else {
                    palette.threads.pending = None;
                }
            }
            PaletteKind::Files => {
                palette.paths.error = None;
                if let Some(source) = palette.source.clone()
                    && !query.is_empty()
                {
                    palette.paths.pending = Some(query.clone());
                    effects.push(Effect::Palette {
                        generation,
                        request: PaletteRequest::Paths { source, query },
                    });
                } else {
                    palette.paths.pending = None;
                }
            }
            _ => {}
        }
        self.refresh_palette_items();
        if let Some(palette) = self.palette.as_mut() {
            palette.selected = 0;
        }
        effects
    }

    pub fn palette_loaded(&mut self, generation: u64, loaded: PaletteLoaded) -> Vec<Effect> {
        match loaded {
            PaletteLoaded::Contributions(result) => {
                match result {
                    Ok(contributions) => self.plugin_contributions = Some(contributions),
                    Err(error) => {
                        tracing::debug!(%error, "plugin contributions unavailable");
                        if self.plugin_contributions.is_none() {
                            self.plugin_contributions = Some(Vec::new());
                        }
                    }
                }
                if let Some(palette) = self.palette.as_mut() {
                    palette.contributions_loading = false;
                }
                if self
                    .palette
                    .as_ref()
                    .is_some_and(|palette| palette.kind == PaletteKind::Commands)
                {
                    self.refresh_palette_items();
                }
            }
            PaletteLoaded::Threads { query, result } => {
                if generation != self.palette_generation {
                    return Vec::new();
                }
                let Some(palette) = self.palette.as_mut() else {
                    return Vec::new();
                };
                if palette.threads.pending.as_deref() != Some(query.as_str()) {
                    return Vec::new();
                }
                palette.threads.pending = None;
                match result {
                    Ok(response) => {
                        palette.threads.query = query;
                        palette.threads.data = Some(response);
                        palette.threads.error = None;
                    }
                    Err(error) => palette.threads.error = Some(error),
                }
                self.refresh_palette_items();
            }
            PaletteLoaded::Paths { query, result } => {
                if generation != self.palette_generation {
                    return Vec::new();
                }
                let Some(palette) = self.palette.as_mut() else {
                    return Vec::new();
                };
                if palette.paths.pending.as_deref() != Some(query.as_str()) {
                    return Vec::new();
                }
                palette.paths.pending = None;
                match result {
                    Ok(response) => {
                        palette.paths.query = query;
                        palette.paths.data = Some(response);
                        palette.paths.error = None;
                    }
                    Err(error) => palette.paths.error = Some(error),
                }
                self.refresh_palette_items();
            }
            PaletteLoaded::File { path, result } => {
                let Some(pane) = self.result_pane.as_mut() else {
                    return Vec::new();
                };
                if pane.generation != generation {
                    return Vec::new();
                }
                match result {
                    Ok(bytes) => {
                        let (lines, note) = pane_lines_for_file(&path, &bytes);
                        pane.body = PaneBody::Lines(lines);
                        pane.footer = note.map(|note| (note, Style::new().fg(Color::Yellow)));
                    }
                    Err(error) => pane.body = PaneBody::Error(error),
                }
            }
            PaletteLoaded::Cli { label, result } => {
                let Some(pane) = self.result_pane.as_mut() else {
                    return Vec::new();
                };
                if pane.generation != generation {
                    return Vec::new();
                }
                match result {
                    Ok(result) => {
                        pane.body = PaneBody::Lines(pane_lines_for_cli(&result));
                        pane.copy_text = Some(result.stdout.clone());
                        let style = if result.exit_code == 0 {
                            Style::new().fg(Color::Green)
                        } else {
                            Style::new().fg(Color::Red)
                        };
                        pane.footer =
                            Some((format!("bb {label} · exit {}", result.exit_code), style));
                    }
                    Err(error) => pane.body = PaneBody::Error(error),
                }
            }
        }
        Vec::new()
    }

    fn open_pane(&mut self, title: String) -> u64 {
        let generation = self.next_palette_generation();
        self.result_pane = Some(ResultPane::new(generation, title));
        generation
    }

    pub(crate) fn open_text_pane(&mut self, title: String, lines: Vec<Vec<Span<'static>>>) {
        let copy = lines
            .iter()
            .map(|line| {
                line.iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        let generation = self.open_pane(title);
        if let Some(pane) = self.result_pane.as_mut()
            && pane.generation == generation
        {
            pane.body = PaneBody::Lines(lines);
            pane.copy_text = Some(copy);
        }
    }

    pub fn close_pane(&mut self, generation: u64) {
        if self
            .result_pane
            .as_ref()
            .is_some_and(|pane| pane.generation == generation)
        {
            self.result_pane = None;
        }
    }

    fn run_plugin_command(&mut self, plugin: PluginCommand, argv: Vec<String>) -> Vec<Effect> {
        let thread_id = self.palette_target_thread();
        let project_id = thread_id.as_deref().and_then(|thread_id| {
            self.sidebar_entry(thread_id)
                .map(|entry| entry.thread.project_id.clone())
                .or_else(|| {
                    self.snapshot
                        .thread(thread_id)
                        .map(|detail| detail.thread.project_id.clone())
                })
        });
        let label = std::iter::once(
            plugin
                .label
                .split(' ')
                .next()
                .unwrap_or_default()
                .to_owned(),
        )
        .chain(argv.iter().cloned())
        .collect::<Vec<_>>()
        .join(" ");
        let generation = self.open_pane(format!("bb {label}"));
        vec![Effect::Palette {
            generation,
            request: PaletteRequest::RunCli {
                plugin_id: plugin.plugin_id,
                request: PluginCliRequest {
                    argv,
                    cwd: None,
                    thread_id,
                    project_id,
                },
            },
        }]
    }

    fn reveal_project(&mut self, project_id: &str) {
        self.sidebar_visible = true;
        self.focus = Focus::Sidebar;
        if !self.filter.is_empty() {
            self.filter.clear();
        }
        self.collapsed.remove(project_id);
        self.rebuild_rows();
        if let Some(index) = self
            .rows
            .iter()
            .position(|row| row.key() == RowKey::Group(project_id.to_owned()))
        {
            self.selected = index;
        }
    }

    fn activate_palette_item(&mut self, item: PaletteItem, kind: PaletteKind) -> Vec<Effect> {
        if kind == PaletteKind::Commands {
            record_recent(&mut self.palette_recents, &item.id);
        }
        let recents_effect = (kind == PaletteKind::Commands)
            .then(|| Effect::SavePaletteRecents(self.palette_recents.clone()));
        let mut effects = match item.target {
            PaletteTarget::Command(command) => {
                self.close_palette();
                crate::app::run_command(self, command)
            }
            PaletteTarget::Plugin(plugin) => {
                let mut palette = Palette::new(PaletteKind::PluginArgs);
                palette.origin = PaletteKind::Commands;
                let mut text = plugin.argv.join(" ");
                if !text.is_empty() {
                    text.push(' ');
                }
                palette.input = LineInput::new(text);
                palette.plugin = Some(plugin);
                self.palette = Some(palette);
                Vec::new()
            }
            PaletteTarget::Thread(thread_id) => {
                self.close_palette();
                let effects = self.open(thread_id);
                self.focus = Focus::Main;
                effects
            }
            PaletteTarget::Project(project_id) => {
                self.close_palette();
                self.reveal_project(&project_id);
                Vec::new()
            }
            PaletteTarget::File(path) => {
                let source = self
                    .palette
                    .as_ref()
                    .and_then(|palette| palette.source.clone());
                self.close_palette();
                let Some(source) = source else {
                    return Vec::new();
                };
                let generation = self.open_pane(path.clone());
                if let Some(pane) = self.result_pane.as_mut() {
                    pane.open_target = source.absolute(&path);
                    pane.copy_text = Some(path.clone());
                }
                vec![Effect::Palette {
                    generation,
                    request: PaletteRequest::File { source, path },
                }]
            }
        };
        effects.extend(recents_effect);
        effects
    }
}

pub(crate) fn handle_palette_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    let Some(palette) = state.palette.as_mut() else {
        return Vec::new();
    };
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    match key.code {
        KeyCode::Esc => {
            state.close_palette();
            return Vec::new();
        }
        KeyCode::Char('c' | 'g') if ctrl => {
            state.close_palette();
            return Vec::new();
        }
        KeyCode::Up | KeyCode::BackTab => {
            palette.move_selection(-1);
            return Vec::new();
        }
        KeyCode::Char('p' | 'k') if ctrl => {
            palette.move_selection(-1);
            return Vec::new();
        }
        KeyCode::Down | KeyCode::Tab => {
            palette.move_selection(1);
            return Vec::new();
        }
        KeyCode::Char('n' | 'j') if ctrl => {
            palette.move_selection(1);
            return Vec::new();
        }
        KeyCode::Char('d') if ctrl => {
            palette.move_selection(PAGE);
            return Vec::new();
        }
        KeyCode::Char('<') if alt => {
            palette.move_selection(-(palette.items.len() as isize));
            return Vec::new();
        }
        KeyCode::Char('>') if alt => {
            palette.move_selection(palette.items.len() as isize);
            return Vec::new();
        }
        KeyCode::PageDown => {
            palette.move_selection(PAGE);
            return Vec::new();
        }
        KeyCode::PageUp => {
            palette.move_selection(-PAGE);
            return Vec::new();
        }
        KeyCode::Backspace if palette.query().is_empty() && palette.kind != palette.origin => {
            let origin = palette.origin;
            return state.open_palette(origin);
        }
        _ => {}
    }
    if (ctrl || alt)
        && let Some(command) = state.keymap.lookup(&key)
        && let Some(kind) = palette_kind_for(command)
    {
        if state
            .palette
            .as_ref()
            .is_some_and(|palette| palette.kind == kind)
        {
            state.close_palette();
            return Vec::new();
        }
        return state.switch_palette(kind);
    }
    let Some(palette) = state.palette.as_mut() else {
        return Vec::new();
    };
    match palette.input.handle_key(key) {
        InputOutcome::Submit => submit_palette(state),
        InputOutcome::Cancel => {
            state.close_palette();
            Vec::new()
        }
        InputOutcome::Edited => state.palette_query_changed(),
        InputOutcome::Moved | InputOutcome::Ignored => Vec::new(),
    }
}

pub fn palette_kind_for(command: Command) -> Option<PaletteKind> {
    [
        PaletteKind::Commands,
        PaletteKind::Threads,
        PaletteKind::Projects,
        PaletteKind::Files,
    ]
    .into_iter()
    .find(|kind| kind.command() == Some(command))
}

fn submit_palette(state: &mut AppState) -> Vec<Effect> {
    let Some(palette) = state.palette.as_ref() else {
        return Vec::new();
    };
    if palette.kind == PaletteKind::PluginArgs {
        let Some(plugin) = palette.plugin.clone() else {
            return Vec::new();
        };
        let argv = match split_argv(palette.query()) {
            Ok(argv) => argv,
            Err(error) => {
                state.toast(error, ToastLevel::Error);
                return Vec::new();
            }
        };
        state.close_palette();
        return state.run_plugin_command(plugin, argv);
    }
    let kind = palette.kind;
    match palette.selected_item().cloned() {
        Some(item) => state.activate_palette_item(item, kind),
        None => Vec::new(),
    }
}

pub(crate) fn handle_pane_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    let Some(pane) = state.result_pane.as_mut() else {
        return Vec::new();
    };
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let page = pane.viewport.get().max(1) as isize;
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter => state.result_pane = None,
        KeyCode::Char('c') if ctrl => state.result_pane = None,
        KeyCode::Char('d') if ctrl => pane.scroll_by(page / 2),
        KeyCode::Char('u') if ctrl => pane.scroll_by(-(page / 2)),
        KeyCode::Char('f') if ctrl => pane.scroll_by(page),
        KeyCode::Char('b') if ctrl => pane.scroll_by(-page),
        KeyCode::Down | KeyCode::Char('j') => pane.scroll_by(1),
        KeyCode::Up | KeyCode::Char('k') => pane.scroll_by(-1),
        KeyCode::PageDown | KeyCode::Char(' ') => pane.scroll_by(page),
        KeyCode::PageUp => pane.scroll_by(-page),
        KeyCode::Home | KeyCode::Char('g') => pane.scroll = 0,
        KeyCode::End | KeyCode::Char('G') => pane.scroll = pane.max_scroll(),
        KeyCode::Left | KeyCode::Char('h') => pane.column = pane.column.saturating_sub(8),
        KeyCode::Right | KeyCode::Char('l') => pane.column += 8,
        KeyCode::Char('0') => pane.column = 0,
        KeyCode::Char('y') => match pane.copy_text.clone() {
            Some(text) if !text.is_empty() => return vec![Effect::Copy(text)],
            _ => state.toast("Nothing to copy", ToastLevel::Info),
        },
        KeyCode::Char('o') => match pane.open_target.clone() {
            Some(target) => return vec![Effect::OpenExternal(target)],
            None => state.toast("Nothing to open", ToastLevel::Info),
        },
        _ => {}
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bb_wire::plugin::PluginCliCommandInfo;

    fn range(start: u32, end: u32) -> HighlightRange {
        HighlightRange { start, end }
    }

    #[test]
    fn plugin_argv_comes_from_usage_words_before_arguments() {
        assert_eq!(
            plugin_argv(
                "thread-list",
                "prefs-list",
                "bb thread-list prefs list [--json]"
            ),
            ["prefs", "list"]
        );
        assert_eq!(
            plugin_argv(
                "concurrency-limit",
                "global",
                "bb concurrency-limit global [<limit>] [--json]"
            ),
            ["global"]
        );
        assert_eq!(
            plugin_argv(
                "taskboard",
                "show",
                "bb taskboard show <linear|github|jira> <locator>"
            ),
            ["show"]
        );
        assert_eq!(
            plugin_argv("x", "fallback", "unrelated usage"),
            ["fallback"]
        );
        let commands = plugin_commands(&[PluginCliContribution {
            plugin_id: "bb-account".into(),
            name: "account".into(),
            summary: "Account".into(),
            commands: vec![PluginCliCommandInfo {
                name: "status".into(),
                summary: String::new(),
                usage: "bb account status [--json]".into(),
            }],
            renders_help: true,
        }]);
        assert_eq!(commands[0].label, "account status");
        assert_eq!(commands[0].summary, "Account");
    }

    #[test]
    fn split_argv_handles_quotes_and_escapes() {
        assert_eq!(
            split_argv(r#"set "hello world" it\'s 'a b' """#).unwrap(),
            ["set", "hello world", "it's", "a b", ""]
        );
        assert!(split_argv("\"open").is_err());
        assert_eq!(split_argv("   ").unwrap(), Vec::<String>::new());
    }

    #[test]
    fn utf16_ranges_convert_to_char_ranges() {
        let text = "😀 fix bug";
        assert_eq!(
            utf16_ranges_to_char_ranges(text, &[range(3, 6), range(0, 1), range(4, 5)]),
            [(0, 1), (2, 5)]
        );
        assert_eq!(
            utf16_ranges_to_char_ranges("abc", &[range(2, 9), range(5, 3)]),
            [(2, 3)]
        );
    }

    #[test]
    fn excerpt_windows_around_first_match_on_word_boundaries() {
        let text = "We spent a long afternoon chasing the flaky websocket reconnect test until the root cause appeared in the scheduler";
        let start = text.find("websocket").unwrap();
        let windowed = window_excerpt(text, &[(start, start + 9)]);
        assert!(windowed.text.starts_with('…'), "{}", windowed.text);
        assert!(windowed.text.ends_with('…'), "{}", windowed.text);
        assert!(
            windowed.text.contains("the flaky websocket"),
            "{}",
            windowed.text
        );
        let highlighted: String = windowed
            .positions
            .iter()
            .map(|index| windowed.text.chars().nth(*index).unwrap())
            .collect();
        assert_eq!(highlighted, "websocket");
        let short = window_excerpt("short text", &[(0, 5)]);
        assert_eq!(short.text, "short text");
        assert_eq!(short.positions, [0, 1, 2, 3, 4]);
    }

    #[test]
    fn title_positions_skip_spaces_and_fail_closed() {
        assert_eq!(title_match_positions("New thread", "n t"), [0, 4]);
        assert!(title_match_positions("New thread", "zz").is_empty());
    }

    fn item(id: &str, title: &str, group: &str) -> PaletteItem {
        PaletteItem {
            id: id.into(),
            group: group.into(),
            title: title.into(),
            positions: Vec::new(),
            detail: None,
            hint: String::new(),
            target: PaletteTarget::Command(Command::Help),
        }
    }

    #[test]
    fn ranking_puts_recents_first_and_breaks_ties_by_recency() {
        let items = vec![
            item("a", "Toggle sidebar", "Layout"),
            item("b", "Toggle panel", "Layout"),
            item("c", "New thread", "Threads"),
        ];
        let recents = vec!["c".to_owned()];
        let empty = rank_actions(items.clone(), "", &recents);
        assert_eq!(empty[0].id, "c");
        assert_eq!(empty[0].group, "Recent");
        let toggles = rank_actions(items.clone(), "toggle", &["b".to_owned()]);
        assert_eq!(toggles[0].id, "b");
        assert_eq!(toggles.len(), 2);
        let by_group = rank_actions(items, "threads", &[]);
        assert_eq!(by_group[0].id, "c");
        let mut recents = vec!["x".to_owned(), "y".to_owned()];
        record_recent(&mut recents, "y");
        assert_eq!(recents, ["y", "x"]);
        for index in 0..20 {
            record_recent(&mut recents, &index.to_string());
        }
        assert_eq!(recents.len(), RECENTS_LIMIT);
    }

    fn entry(id: &str, project: &str, title: &str, updated_at: i64) -> ThreadListEntry {
        let mut entry = ThreadListEntry::default();
        entry.thread.id = id.into();
        entry.thread.project_id = project.into();
        entry.thread.title = Some(title.into());
        entry.thread.updated_at = updated_at;
        entry
    }

    #[test]
    fn thread_rows_merge_local_and_server_results() {
        let threads = vec![
            entry("t1", "p1", "Fix websocket reconnect", 10),
            entry("t2", "p2", "Docs cleanup", 30),
            entry("t3", "p1", "Unrelated", 20),
        ];
        let project_names = HashMap::from([
            ("p1".to_owned(), "server".to_owned()),
            ("p2".to_owned(), "web socket".to_owned()),
        ]);
        let personal = HashSet::new();
        let mut response = ThreadSearchResponse::default();
        response
            .active
            .results
            .push(bb_wire::thread::ThreadSearchResult {
                thread: entry("t3", "p1", "Unrelated", 20),
                matches: vec![ThreadSearchMatch {
                    source_kind: ThreadSearchSourceKind::UserMessage,
                    text: "the websocket dropped".into(),
                    highlight_ranges: vec![range(4, 13)],
                    source_seq: Some(4),
                }],
            });
        response
            .archived
            .results
            .push(bb_wire::thread::ThreadSearchResult {
                thread: entry("t9", "p1", "Old websocket work", 5),
                matches: Vec::new(),
            });
        let input = ThreadSearchInput {
            threads: &threads,
            project_names: &project_names,
            personal_ids: &personal,
            query: "websocket",
            response: Some(&response),
        };
        let (recent, rows) = build_thread_rows(&input);
        assert!(!recent);
        let ids: Vec<&str> = rows.iter().map(|row| row.thread_id.as_str()).collect();
        assert_eq!(ids, ["t1", "t3", "t9"]);
        assert_eq!(
            rows[1].excerpt.as_ref().unwrap().text,
            "the websocket dropped"
        );
        assert!(rows[2].archived);
        let project_query = ThreadSearchInput {
            query: "web",
            response: None,
            ..input
        };
        let (_, rows) = build_thread_rows(&project_query);
        let ids: Vec<&str> = rows.iter().map(|row| row.thread_id.as_str()).collect();
        assert_eq!(ids, ["t1", "t2"]);
        assert_eq!(rows[1].project_ranges, [(0, 3)]);
        let recent_input = ThreadSearchInput {
            query: "  ",
            ..project_query
        };
        let (recent, rows) = build_thread_rows(&recent_input);
        assert!(recent);
        assert_eq!(rows[0].thread_id, "t2");
    }
}
