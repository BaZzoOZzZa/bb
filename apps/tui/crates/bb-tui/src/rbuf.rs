use std::cell::RefCell;
use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use regex::{Regex, RegexBuilder};
use unicode_width::UnicodeWidthChar;

use crate::app::{AppState, Effect, Focus, Mode};
use crate::evil::RegisterValue;
use crate::input::{InputOutcome, LineInput};
use crate::keymap::ScreenPos;
use crate::palette::PaneBody;
use crate::panel::PanelTab;
use crate::sidebar::{RowKey, SidebarRow};
use crate::timeline::{Frame as TimelineFrame, LineCursor, Scroll};
use crate::vi::{self, CharAct, Motion, Object, Operator, Range, Target};

const MAX_JUMPS: usize = 100;
const MAX_HISTORY: usize = 100;
const SEARCH_STYLE: Style = Style::new().bg(Color::Yellow).fg(Color::Black);
const VISUAL_STYLE: Style = Style::new().bg(Color::Indexed(60)).fg(Color::White);
pub const CURSOR_LINE: Style = Style::new().bg(Color::Indexed(236));

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Buf {
    Sidebar,
    Timeline,
    Diff,
    File,
    Terminal,
    Pane,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum BufId {
    Sidebar,
    Timeline(String),
    Diff,
    File(String),
    Terminal(String),
    Pane(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Anchor {
    Line {
        line: usize,
        col: usize,
    },
    Row {
        key: String,
        offset: usize,
        col: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Batch {
    Archive,
    Read,
    Pin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ViewOp {
    Move(Motion),
    Page { down: bool, half: bool },
    Scroll(i8),
    Align(ScreenPos),
    Screen(ScreenPos),
    Visual { line: bool },
    Reselect,
    VisualSwap,
    Yank(Target),
    YankRow { around: bool },
    VisualYank { line: bool },
    VisualObject(Target),
    VisualRow { around: bool },
    Search { forward: bool },
    SearchNext { reverse: bool },
    SearchWord { forward: bool, whole: bool },
    JumpOlder,
    JumpNewer,
    Batch(Batch),
}

impl ViewOp {
    pub fn description(self) -> String {
        match self {
            Self::Move(motion) => {
                let label = motion.label();
                let mut chars = label.chars();
                chars.next().map_or_else(String::new, |first| {
                    first.to_uppercase().chain(chars).collect()
                })
            }
            Self::Page {
                down: true,
                half: true,
            } => "Half page down".into(),
            Self::Page {
                down: false,
                half: true,
            } => "Half page up".into(),
            Self::Page { down: true, .. } => "Page down".into(),
            Self::Page { .. } => "Page up".into(),
            Self::Scroll(delta) if delta > 0 => "Scroll one line down".into(),
            Self::Scroll(_) => "Scroll one line up".into(),
            Self::Align(ScreenPos::Top) => "Cursor line to the top".into(),
            Self::Align(ScreenPos::Middle) => "Cursor line to the middle".into(),
            Self::Align(ScreenPos::Bottom) => "Cursor line to the bottom".into(),
            Self::Screen(ScreenPos::Top) => "Top of the window".into(),
            Self::Screen(ScreenPos::Middle) => "Middle of the window".into(),
            Self::Screen(ScreenPos::Bottom) => "Bottom of the window".into(),
            Self::Visual { line: false } => "Visual selection".into(),
            Self::Visual { line: true } => "Visual line selection".into(),
            Self::Reselect => "Reselect the last visual selection".into(),
            Self::VisualSwap => "Go to the other end of the selection".into(),
            Self::Yank(Target::Line) => "Yank the line".into(),
            Self::Yank(Target::Motion(motion)) => format!("Yank: {}", motion.label()),
            Self::Yank(Target::Inner(object)) => format!("Yank inner {}", object.label()),
            Self::Yank(Target::Around(object)) => format!("Yank around {}", object.label()),
            Self::YankRow { around: false } => "Yank the row body".into(),
            Self::YankRow { around: true } => "Yank the row with its header".into(),
            Self::VisualYank { line: false } => "Yank the selection".into(),
            Self::VisualYank { line: true } => "Yank the selected lines".into(),
            Self::VisualObject(Target::Inner(object)) => format!("Select inner {}", object.label()),
            Self::VisualObject(Target::Around(object)) => {
                format!("Select around {}", object.label())
            }
            Self::VisualObject(_) => "Select".into(),
            Self::VisualRow { around: false } => "Select the row body".into(),
            Self::VisualRow { around: true } => "Select the row with its header".into(),
            Self::Search { forward: true } => "Search forward".into(),
            Self::Search { forward: false } => "Search backward".into(),
            Self::SearchNext { reverse: false } => "Next search match".into(),
            Self::SearchNext { reverse: true } => "Previous search match".into(),
            Self::SearchWord { forward: true, .. } => "Search the word under the cursor".into(),
            Self::SearchWord { forward: false, .. } => {
                "Search the word under the cursor backward".into()
            }
            Self::JumpOlder => "Jump back".into(),
            Self::JumpNewer => "Jump forward".into(),
            Self::Batch(Batch::Archive) => "Archive the selected threads".into(),
            Self::Batch(Batch::Read) => "Mark the selected threads read".into(),
            Self::Batch(Batch::Pin) => "Pin the selected threads".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Visual {
    pub buf: BufId,
    pub anchor: Anchor,
    pub line: bool,
}

#[derive(Debug, Clone)]
pub struct Search {
    pub pattern: String,
    pub regex: Regex,
    pub forward: bool,
    pub highlight: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Origin {
    buf: Buf,
    line: usize,
    col: usize,
    top: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchLine {
    pub input: LineInput,
    pub forward: bool,
    history: Option<usize>,
    origin: Origin,
}

#[derive(Debug, Default)]
struct Jumps {
    list: Vec<Anchor>,
    index: usize,
}

#[derive(Debug)]
pub struct Views {
    pub visual: Option<Visual>,
    pub search: Option<Search>,
    pub preview: Option<Regex>,
    pub history: Vec<String>,
    pub wrapscan: bool,
    pub widths: RefCell<HashMap<Buf, usize>>,
    last_visual: HashMap<BufId, (Anchor, Anchor, bool)>,
    marks: HashMap<BufId, HashMap<char, Anchor>>,
    global_marks: HashMap<char, (String, Anchor)>,
    jumps: HashMap<BufId, Jumps>,
    cols: HashMap<BufId, (usize, usize)>,
    want: Option<(Buf, usize)>,
}

impl Default for Views {
    fn default() -> Self {
        Self {
            visual: None,
            search: None,
            preview: None,
            history: Vec::new(),
            wrapscan: true,
            widths: RefCell::new(HashMap::new()),
            last_visual: HashMap::new(),
            marks: HashMap::new(),
            global_marks: HashMap::new(),
            jumps: HashMap::new(),
            cols: HashMap::new(),
            want: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Doc {
    pub lines: Vec<String>,
    chars: Vec<char>,
    starts: Vec<usize>,
}

impl Doc {
    pub fn new(mut lines: Vec<String>) -> Self {
        if lines.is_empty() {
            lines.push(String::new());
        }
        let mut chars = Vec::new();
        let mut starts = Vec::with_capacity(lines.len());
        for (index, line) in lines.iter_mut().enumerate() {
            if line.contains('\n') {
                *line = line.replace('\n', " ");
            }
            if index > 0 {
                chars.push('\n');
            }
            starts.push(chars.len());
            chars.extend(line.chars());
        }
        Self {
            lines,
            chars,
            starts,
        }
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    fn end(&self, line: usize) -> usize {
        self.starts
            .get(line + 1)
            .map_or(self.chars.len(), |start| start - 1)
    }

    pub fn line_len(&self, line: usize) -> usize {
        let line = line.min(self.len() - 1);
        self.end(line) - self.starts[line]
    }

    pub fn pos(&self, line: usize, col: usize) -> usize {
        let line = line.min(self.len() - 1);
        self.starts[line] + col.min(self.line_len(line))
    }

    pub fn at(&self, pos: usize) -> (usize, usize) {
        let line = self.starts.partition_point(|start| *start <= pos).max(1) - 1;
        (line, pos - self.starts[line])
    }

    pub fn clamp(&self, line: usize, col: usize) -> (usize, usize) {
        let line = line.min(self.len() - 1);
        let len = self.line_len(line);
        (line, if len == 0 { 0 } else { col.min(len - 1) })
    }

    fn text(&self, start: usize, end: usize) -> String {
        self.chars[start.min(self.chars.len())..end.min(self.chars.len())]
            .iter()
            .collect()
    }

    fn first_non_blank(&self, line: usize) -> usize {
        self.lines[line.min(self.len() - 1)]
            .chars()
            .position(|ch| !ch.is_whitespace())
            .unwrap_or(0)
    }
}

pub fn compile(pattern: &str) -> Option<Regex> {
    let mut text = pattern.to_owned();
    let mut insensitive = None;
    if text.contains("\\c") {
        insensitive = Some(true);
        text = text.replace("\\c", "");
    }
    if text.contains("\\C") {
        insensitive = Some(false);
        text = text.replace("\\C", "");
    }
    if text.is_empty() {
        return None;
    }
    let insensitive = insensitive.unwrap_or_else(|| !text.chars().any(char::is_uppercase));
    RegexBuilder::new(&text)
        .case_insensitive(insensitive)
        .build()
        .or_else(|_| {
            RegexBuilder::new(&regex::escape(&text))
                .case_insensitive(insensitive)
                .build()
        })
        .ok()
}

fn char_col(text: &str, byte: usize) -> usize {
    text[..byte].chars().count()
}

fn line_matches(regex: &Regex, text: &str) -> Vec<(usize, usize)> {
    regex
        .find_iter(text)
        .filter(|found| found.end() > found.start())
        .map(|found| (char_col(text, found.start()), char_col(text, found.end())))
        .collect()
}

pub fn find_match(
    doc: &Doc,
    regex: &Regex,
    line: usize,
    col: usize,
    forward: bool,
    wrap: bool,
) -> Option<((usize, usize), bool)> {
    let total = doc.len();
    if forward {
        for step in 0..=total {
            let index = line + step;
            let wrapped = index >= total;
            if wrapped && !wrap {
                return None;
            }
            let current = index % total;
            let found = line_matches(regex, &doc.lines[current])
                .into_iter()
                .map(|(start, _)| start)
                .find(|start| step > 0 || *start > col);
            if let Some(start) = found {
                return Some(((current, start), wrapped));
            }
        }
    } else {
        for step in 0..=total {
            let wrapped = step > line;
            if wrapped && !wrap {
                return None;
            }
            let current = (line + total * 2 - step) % total;
            let found = line_matches(regex, &doc.lines[current])
                .into_iter()
                .map(|(start, _)| start)
                .rev()
                .find(|start| step > 0 || *start < col);
            if let Some(start) = found {
                return Some(((current, start), wrapped));
            }
        }
    }
    None
}

fn match_index(
    doc: &Doc,
    regex: &Regex,
    line: usize,
    col: usize,
    lines_only: bool,
) -> (usize, usize) {
    let mut index = 0;
    let mut total = 0;
    for (current, text) in doc.lines.iter().enumerate() {
        let mut found = line_matches(regex, text);
        if lines_only {
            found.truncate(1);
        }
        for (start, _) in found {
            total += 1;
            if (current, start) <= (line, col) {
                index = total;
            }
        }
    }
    (index, total)
}

fn row_key_text(key: &RowKey) -> String {
    match key {
        RowKey::Group(key) => format!("g:{key}"),
        RowKey::Thread(id) => format!("t:{id}"),
    }
}

pub fn has_columns(buf: Buf) -> bool {
    buf != Buf::Sidebar
}

pub fn focused(state: &AppState) -> Option<Buf> {
    if let Some(pane) = &state.result_pane {
        return matches!(pane.body, PaneBody::Lines(_)).then_some(Buf::Pane);
    }
    if state.settings.is_some()
        || state.automations.is_some()
        || matches!(state.mode, Mode::Notifications { .. })
    {
        return None;
    }
    match state.focus {
        Focus::Sidebar if state.sidebar_visible => Some(Buf::Sidebar),
        Focus::Main if state.timeline().is_some() => Some(Buf::Timeline),
        Focus::Panel if state.panel_visible => match state.panel.tab {
            PanelTab::Diff => state.panel.document().map(|_| Buf::Diff),
            PanelTab::File => state
                .panel
                .preview
                .as_ref()
                .filter(|preview| preview.body.loaded().is_some())
                .map(|_| Buf::File),
            PanelTab::Terminal if !state.terminal_input_active() => {
                state.terminals.active_view().map(|_| Buf::Terminal)
            }
            _ => None,
        },
        _ => None,
    }
}

pub fn id(state: &AppState, buf: Buf) -> BufId {
    match buf {
        Buf::Sidebar => BufId::Sidebar,
        Buf::Timeline => BufId::Timeline(state.open_thread.clone().unwrap_or_default()),
        Buf::Diff => BufId::Diff,
        Buf::File => BufId::File(
            state
                .panel
                .preview
                .as_ref()
                .map(|preview| preview.path.clone())
                .unwrap_or_default(),
        ),
        Buf::Terminal => BufId::Terminal(state.terminals.active.clone().unwrap_or_default()),
        Buf::Pane => BufId::Pane(
            state
                .result_pane
                .as_ref()
                .map(|pane| pane.title.clone())
                .unwrap_or_default(),
        ),
    }
}

fn spans_text(spans: &[ratatui::text::Span<'_>]) -> String {
    spans.iter().map(|span| span.content.as_ref()).collect()
}

fn timeline_frame(state: &AppState) -> TimelineFrame {
    TimelineFrame {
        now_ms: state.now_ms,
        tick: state.frame,
    }
}

pub fn sidebar_label(row: &SidebarRow) -> String {
    match row {
        SidebarRow::Group(group) => group.label.clone(),
        SidebarRow::Thread(thread) => thread.title.clone(),
    }
}

pub fn doc(state: &AppState, buf: Buf) -> Doc {
    let lines = match buf {
        Buf::Sidebar => state.rows.iter().map(sidebar_label).collect(),
        Buf::Timeline => state
            .timeline()
            .map(|timeline| timeline.plain_lines(timeline_frame(state)))
            .unwrap_or_default(),
        Buf::Diff => state
            .panel
            .document()
            .map(|document| {
                (0..document.row_count())
                    .map(|index| crate::panel_ui::diff_row_text(document, index).0)
                    .collect()
            })
            .unwrap_or_default(),
        Buf::File => state
            .panel
            .preview
            .as_ref()
            .and_then(|preview| preview.body.loaded())
            .map(|lines| lines.iter().map(|spans| spans_text(spans)).collect())
            .unwrap_or_default(),
        Buf::Terminal => state
            .terminals
            .active_view()
            .map(|view| view.history_lines().0)
            .unwrap_or_default(),
        Buf::Pane => match state.result_pane.as_ref().map(|pane| &pane.body) {
            Some(PaneBody::Lines(lines)) => lines.iter().map(|spans| spans_text(spans)).collect(),
            _ => Vec::new(),
        },
    };
    Doc::new(lines)
}

fn timeline_starts(state: &AppState) -> Option<(Vec<usize>, usize)> {
    let timeline = state.timeline()?;
    let (width, _) = timeline.viewport_size();
    Some(timeline.item_starts(width))
}

fn sidebar_height(state: &AppState) -> usize {
    state
        .mouse_areas
        .sidebar
        .get()
        .map_or(20, |(area, _)| usize::from(area.height))
        .max(1)
}

fn terminal_top(state: &AppState) -> Option<(usize, usize)> {
    let view = state.terminals.active_view()?;
    let max = view.max_scrollback();
    let (rows, _) = view.screen.screen().size();
    Some((max - view.scrollback().min(max), usize::from(rows.max(1))))
}

pub fn view(state: &AppState, buf: Buf) -> (usize, usize) {
    match buf {
        Buf::Sidebar => (state.sidebar_offset.get(), sidebar_height(state)),
        Buf::Timeline => match (state.timeline(), timeline_starts(state)) {
            (Some(timeline), Some((starts, _))) => {
                (timeline.top_line(&starts), timeline.viewport_size().1)
            }
            _ => (0, 1),
        },
        Buf::Diff => state.panel.document().map_or((0, 1), |document| {
            (document.scroll, document.viewport.get().max(1))
        }),
        Buf::File => state.panel.preview.as_ref().map_or((0, 1), |preview| {
            (preview.scroll, preview.viewport.get().max(1))
        }),
        Buf::Terminal => terminal_top(state).unwrap_or((0, 1)),
        Buf::Pane => state
            .result_pane
            .as_ref()
            .map_or((0, 1), |pane| (pane.scroll, pane.viewport.get().max(1))),
    }
}

fn stored_col(state: &AppState, buf: Buf, line: usize) -> usize {
    match state.views.cols.get(&id(state, buf)) {
        Some((stored, col)) if *stored == line => *col,
        _ => 0,
    }
}

pub fn raw_cursor(state: &AppState, buf: Buf) -> Option<(usize, usize)> {
    match buf {
        Buf::Sidebar => Some((state.selected, 0)),
        Buf::Timeline => {
            let (starts, total) = timeline_starts(state)?;
            state.timeline()?.cursor_position(&starts, total)
        }
        Buf::Diff => {
            let line = state.panel.document()?.cursor;
            Some((line, stored_col(state, buf, line)))
        }
        Buf::File => {
            let line = state.panel.preview.as_ref()?.cursor;
            Some((line, stored_col(state, buf, line)))
        }
        Buf::Pane => {
            let line = state.result_pane.as_ref()?.cursor;
            Some((line, stored_col(state, buf, line)))
        }
        Buf::Terminal => state.views.cols.get(&id(state, buf)).copied(),
    }
}

pub fn cursor(state: &AppState, buf: Buf, doc: &Doc) -> (usize, usize) {
    let (line, col) = raw_cursor(state, buf).unwrap_or_else(|| {
        let (top, height) = view(state, buf);
        match buf {
            Buf::Timeline
                if state
                    .timeline()
                    .is_some_and(|timeline| timeline.scroll == Scroll::Bottom) =>
            {
                ((top + height).min(doc.len()).saturating_sub(1), 0)
            }
            Buf::Terminal => {
                let row = state.terminals.active_view().map_or(0, |view| {
                    usize::from(view.screen.screen().cursor_position().0)
                });
                (top + row, 0)
            }
            _ => (top, 0),
        }
    });
    doc.clamp(line, col)
}

fn display_x(text: &str, col: usize) -> usize {
    text.chars()
        .take(col)
        .map(|ch| ch.width().unwrap_or(0))
        .sum()
}

fn follow_column(column: &mut usize, x: usize, width: usize) {
    let width = width.max(1);
    if x < *column {
        *column = x;
    } else if x >= *column + width {
        *column = x + 1 - width;
    }
}

fn keep_line_visible(scroll: &mut usize, line: usize, height: usize) {
    let height = height.max(1);
    if line < *scroll {
        *scroll = line;
    } else if line >= *scroll + height {
        *scroll = line + 1 - height;
    }
}

pub(crate) fn place(
    state: &mut AppState,
    buf: Buf,
    doc: &Doc,
    line: usize,
    col: usize,
) -> Vec<Effect> {
    let (line, col) = doc.clamp(line, col);
    let x = display_x(&doc.lines[line], col);
    let width = state.views.widths.borrow().get(&buf).copied().unwrap_or(80);
    let key = id(state, buf);
    match buf {
        Buf::Sidebar => {
            state.selected = line.min(state.rows.len().saturating_sub(1));
            let height = sidebar_height(state);
            let mut offset = state.sidebar_offset.get();
            keep_line_visible(&mut offset, state.selected, height);
            state.sidebar_offset.set(offset);
            Vec::new()
        }
        Buf::Timeline => {
            let Some((starts, total)) = timeline_starts(state) else {
                return Vec::new();
            };
            state.with_timeline(|timeline| {
                timeline.place_cursor(&starts, total, line, col);
                timeline.auto_load_older()
            })
        }
        Buf::Diff => {
            state.views.cols.insert(key, (line, col));
            if let Some(document) = state.panel.document_mut() {
                document.set_cursor(line);
                if matches!(document.row(line), Some(crate::git_diff::DiffRow::Line(..))) {
                    follow_column(&mut document.column, x, width);
                }
            }
            state.diff_loads()
        }
        Buf::File => {
            state.views.cols.insert(key, (line, col));
            if let Some(preview) = state.panel.preview.as_mut() {
                preview.cursor = line;
                keep_line_visible(&mut preview.scroll, line, preview.viewport.get());
                follow_column(&mut preview.column, x, width);
            }
            Vec::new()
        }
        Buf::Pane => {
            state.views.cols.insert(key, (line, col));
            if let Some(pane) = state.result_pane.as_mut() {
                pane.cursor = line;
                keep_line_visible(&mut pane.scroll, line, pane.viewport.get());
                follow_column(&mut pane.column, x, width);
            }
            Vec::new()
        }
        Buf::Terminal => {
            state.views.cols.insert(key, (line, col));
            if let Some((top, height)) = terminal_top(state) {
                let mut next = top;
                keep_line_visible(&mut next, line, height);
                if next != top {
                    set_terminal_top(state, next);
                }
            }
            Vec::new()
        }
    }
}

fn set_terminal_top(state: &mut AppState, top: usize) {
    if let Some(view) = state.terminals.active_view_mut() {
        let max = view.max_scrollback();
        view.scroll_to(max.saturating_sub(top));
    }
}

fn set_top(state: &mut AppState, buf: Buf, top: usize, total: usize) -> Vec<Effect> {
    let (_, height) = view(state, buf);
    let top = top.min(total.saturating_sub(height));
    match buf {
        Buf::Sidebar => {
            state.sidebar_offset.set(top);
            Vec::new()
        }
        Buf::Timeline => {
            let Some((starts, _)) = timeline_starts(state) else {
                return Vec::new();
            };
            state.with_timeline(|timeline| {
                timeline.scroll_to_line(&starts, top);
                timeline.auto_load_older()
            })
        }
        Buf::Diff => {
            if let Some(document) = state.panel.document_mut() {
                document.scroll = top;
            }
            state.diff_loads()
        }
        Buf::File => {
            if let Some(preview) = state.panel.preview.as_mut() {
                preview.scroll = top;
            }
            Vec::new()
        }
        Buf::Pane => {
            if let Some(pane) = state.result_pane.as_mut() {
                pane.scroll = top;
            }
            Vec::new()
        }
        Buf::Terminal => {
            set_terminal_top(state, top);
            Vec::new()
        }
    }
}

fn anchor(state: &AppState, buf: Buf, line: usize, col: usize) -> Anchor {
    match buf {
        Buf::Sidebar => match state.rows.get(line) {
            Some(row) => Anchor::Row {
                key: row_key_text(&row.key()),
                offset: 0,
                col: 0,
            },
            None => Anchor::Line { line, col },
        },
        Buf::Timeline => {
            let found =
                state
                    .timeline()
                    .zip(timeline_starts(state))
                    .and_then(|(timeline, (starts, _))| {
                        let index = starts
                            .partition_point(|start| *start <= line)
                            .checked_sub(1)?;
                        Some(Anchor::Row {
                            key: timeline.items.get(index)?.key.clone(),
                            offset: line - starts[index],
                            col,
                        })
                    });
            found.unwrap_or(Anchor::Line { line, col })
        }
        _ => Anchor::Line { line, col },
    }
}

fn resolve(state: &AppState, buf: Buf, anchor: &Anchor) -> Option<(usize, usize)> {
    match (buf, anchor) {
        (_, Anchor::Line { line, col }) => Some((*line, *col)),
        (Buf::Sidebar, Anchor::Row { key, .. }) => state
            .rows
            .iter()
            .position(|row| row_key_text(&row.key()) == *key)
            .map(|line| (line, 0)),
        (Buf::Timeline, Anchor::Row { key, offset, col }) => {
            let timeline = state.timeline()?;
            let (starts, _) = timeline_starts(state)?;
            let index = timeline.resolve_item(key)?;
            let (width, _) = timeline.viewport_size();
            let height = timeline.item_height(index, width).max(1);
            let offset = if timeline.items[index].key == *key {
                (*offset).min(height - 1)
            } else {
                0
            };
            Some((starts[index] + offset, *col))
        }
        _ => None,
    }
}

fn want_col(state: &AppState, buf: Buf, col: usize) -> usize {
    match state.views.want {
        Some((stored, want)) if stored == buf => want,
        _ => col,
    }
}

fn is_jump(motion: Motion) -> bool {
    matches!(
        motion,
        Motion::FirstLine
            | Motion::LastLine
            | Motion::MatchPair
            | Motion::ParagraphForward
            | Motion::ParagraphBackward
            | Motion::SentenceForward
            | Motion::SentenceBackward
    )
}

fn is_horizontal(motion: Motion) -> bool {
    !matches!(
        motion,
        Motion::Up
            | Motion::Down
            | Motion::FirstLine
            | Motion::LastLine
            | Motion::ParagraphForward
            | Motion::ParagraphBackward
    )
}

fn motion_target(
    state: &mut AppState,
    buf: Buf,
    doc: &Doc,
    (line, col): (usize, usize),
    motion: Motion,
    count: Option<usize>,
) -> Option<(usize, usize)> {
    let columns = has_columns(buf);
    match motion {
        Motion::Up | Motion::Down => {
            let times = count.unwrap_or(1).max(1);
            let target = if motion == Motion::Down {
                (line + times).min(doc.len() - 1)
            } else {
                line.saturating_sub(times)
            };
            let want = if columns {
                want_col(state, buf, col)
            } else {
                0
            };
            state.views.want = Some((buf, want));
            Some(doc.clamp(target, want))
        }
        Motion::FirstLine | Motion::LastLine if !columns => {
            let target = match (motion, count) {
                (_, Some(count)) => count.saturating_sub(1),
                (Motion::FirstLine, None) => 0,
                _ => doc.len() - 1,
            };
            Some(doc.clamp(target, 0))
        }
        _ if !columns && is_horizontal(motion) => None,
        _ => {
            let pos = doc.pos(line, col);
            let next = vi::motion(&doc.chars, pos, motion, count, &mut state.keys.last_find)?;
            let (target, target_col) = doc.at(next);
            let clamped = doc.clamp(target, if columns { target_col } else { 0 });
            let want = if motion == Motion::LineEnd {
                usize::MAX
            } else {
                clamped.1
            };
            state.views.want = Some((buf, want));
            Some(clamped)
        }
    }
}

fn push_jump(state: &mut AppState, buf: Buf, line: usize, col: usize) {
    let mark = anchor(state, buf, line, col);
    let key = id(state, buf);
    state
        .views
        .marks
        .entry(key.clone())
        .or_default()
        .insert('\'', mark.clone());
    let jumps = state.views.jumps.entry(key).or_default();
    jumps.list.retain(|existing| *existing != mark);
    jumps.list.push(mark);
    if jumps.list.len() > MAX_JUMPS {
        jumps.list.remove(0);
    }
    jumps.index = jumps.list.len();
}

pub(crate) fn note_jump(state: &mut AppState) {
    if let Some(buf) = focused(state) {
        let doc = doc(state, buf);
        let (line, col) = cursor(state, buf, &doc);
        push_jump(state, buf, line, col);
    }
}

fn move_cursor(
    state: &mut AppState,
    buf: Buf,
    motion: Motion,
    count: Option<usize>,
) -> Vec<Effect> {
    let doc = doc(state, buf);
    let current = cursor(state, buf, &doc);
    let Some((line, col)) = motion_target(state, buf, &doc, current, motion, count) else {
        return Vec::new();
    };
    if is_jump(motion) && line != current.0 {
        push_jump(state, buf, current.0, current.1);
    }
    place(state, buf, &doc, line, col)
}

fn page(
    state: &mut AppState,
    buf: Buf,
    down: bool,
    half: bool,
    count: Option<usize>,
) -> Vec<Effect> {
    let doc = doc(state, buf);
    let (line, col) = cursor(state, buf, &doc);
    let (top, height) = view(state, buf);
    let amount = if half {
        count.unwrap_or(height / 2).max(1)
    } else {
        height.saturating_sub(2).max(1) * count.unwrap_or(1).max(1)
    };
    let total = doc.len();
    let next_top = if down {
        top + amount
    } else {
        top.saturating_sub(amount)
    };
    let next_line = if down {
        (line + amount).min(total - 1)
    } else {
        line.saturating_sub(amount)
    };
    let mut effects = set_top(state, buf, next_top, total);
    let want = want_col(state, buf, col);
    effects.extend(place(state, buf, &doc, next_line, want));
    effects
}

fn scroll(state: &mut AppState, buf: Buf, delta: i8, count: Option<usize>) -> Vec<Effect> {
    let doc = doc(state, buf);
    let (line, col) = cursor(state, buf, &doc);
    let (top, height) = view(state, buf);
    let amount = count.unwrap_or(1).max(1);
    let total = doc.len();
    let next_top = if delta > 0 {
        (top + amount).min(total.saturating_sub(height))
    } else {
        top.saturating_sub(amount)
    };
    let mut effects = set_top(state, buf, next_top, total);
    let (top, height) = view(state, buf);
    let next_line = line.clamp(top, (top + height).min(total).saturating_sub(1).max(top));
    let want = want_col(state, buf, col);
    effects.extend(place(state, buf, &doc, next_line, want));
    effects
}

fn align(state: &mut AppState, buf: Buf, position: ScreenPos, count: Option<usize>) -> Vec<Effect> {
    let doc = doc(state, buf);
    let (mut line, mut col) = cursor(state, buf, &doc);
    if let Some(count) = count {
        line = count.saturating_sub(1).min(doc.len() - 1);
        col = doc.first_non_blank(line);
    }
    let (_, height) = view(state, buf);
    let top = match position {
        ScreenPos::Top => line,
        ScreenPos::Middle => line.saturating_sub(height / 2),
        ScreenPos::Bottom => (line + 1).saturating_sub(height),
    };
    let mut effects = place(state, buf, &doc, line, col);
    effects.extend(set_top(state, buf, top, doc.len()));
    effects
}

fn screen(
    state: &mut AppState,
    buf: Buf,
    position: ScreenPos,
    count: Option<usize>,
) -> Vec<Effect> {
    let doc = doc(state, buf);
    let (top, height) = view(state, buf);
    let visible = height.min(doc.len().saturating_sub(top)).max(1);
    let skip = count.unwrap_or(1).max(1) - 1;
    let line = match position {
        ScreenPos::Top => top + skip.min(visible - 1),
        ScreenPos::Middle => top + (visible - 1) / 2,
        ScreenPos::Bottom => top + (visible - 1).saturating_sub(skip),
    };
    let col = if has_columns(buf) {
        doc.first_non_blank(line)
    } else {
        0
    };
    place(state, buf, &doc, line, col)
}

fn current_anchor(state: &AppState, buf: Buf, doc: &Doc) -> Anchor {
    let (line, col) = cursor(state, buf, doc);
    anchor(state, buf, line, col)
}

fn toggle_visual(state: &mut AppState, buf: Buf, line: bool) -> Vec<Effect> {
    let key = id(state, buf);
    let line = line || !has_columns(buf);
    match state.views.visual.as_mut() {
        Some(visual) if visual.buf == key && visual.line == line => end_visual(state),
        Some(visual) if visual.buf == key => visual.line = line,
        _ => {
            let doc = doc(state, buf);
            let anchor = current_anchor(state, buf, &doc);
            state.views.visual = Some(Visual {
                buf: key,
                anchor,
                line,
            });
        }
    }
    Vec::new()
}

pub(crate) fn end_visual(state: &mut AppState) {
    let Some(visual) = state.views.visual.take() else {
        return;
    };
    let Some(buf) = focused(state).filter(|buf| id(state, *buf) == visual.buf) else {
        return;
    };
    let doc = doc(state, buf);
    let head = current_anchor(state, buf, &doc);
    if let (Some(start), Some(end)) = (
        resolve(state, buf, &visual.anchor),
        resolve(state, buf, &head),
    ) {
        let (first, last) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        let marks = state.views.marks.entry(visual.buf.clone()).or_default();
        marks.insert(
            '<',
            Anchor::Line {
                line: first.0,
                col: first.1,
            },
        );
        marks.insert(
            '>',
            Anchor::Line {
                line: last.0,
                col: last.1,
            },
        );
    }
    state
        .views
        .last_visual
        .insert(visual.buf, (visual.anchor, head, visual.line));
}

pub(crate) fn escape(state: &mut AppState) -> bool {
    if state.views.visual.is_some() {
        end_visual(state);
        return true;
    }
    if focused(state).is_some()
        && let Some(search) = state.views.search.as_mut()
        && search.highlight
    {
        search.highlight = false;
        return true;
    }
    false
}

pub(crate) fn nohlsearch(state: &mut AppState) {
    if let Some(search) = state.views.search.as_mut() {
        search.highlight = false;
    }
}

pub(crate) fn sync_focus(state: &mut AppState) {
    if let Some(visual) = &state.views.visual
        && focused(state).map(|buf| id(state, buf)).as_ref() != Some(&visual.buf)
    {
        state.views.visual = None;
    }
}

pub fn visual_active(state: &AppState) -> Option<bool> {
    let visual = state.views.visual.as_ref()?;
    let buf = focused(state)?;
    (id(state, buf) == visual.buf).then_some(visual.line)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub start: (usize, usize),
    pub end: (usize, usize),
    pub line: bool,
}

fn selection(state: &AppState, buf: Buf, doc: Option<&Doc>) -> Option<Selection> {
    let visual = state.views.visual.as_ref()?;
    if id(state, buf) != visual.buf {
        return None;
    }
    let start = resolve(state, buf, &visual.anchor)?;
    let head = match doc {
        Some(doc) => cursor(state, buf, doc),
        None => raw_cursor(state, buf)?,
    };
    let (start, end) = if start <= head {
        (start, head)
    } else {
        (head, start)
    };
    Some(Selection {
        start,
        end,
        line: visual.line,
    })
}

fn reselect(state: &mut AppState, buf: Buf) -> Vec<Effect> {
    let key = id(state, buf);
    let Some((start, head, line)) = state.views.last_visual.get(&key).cloned() else {
        state.keys.error("E20: Mark not set");
        return Vec::new();
    };
    let Some((line_no, col)) = resolve(state, buf, &head) else {
        return Vec::new();
    };
    state.views.visual = Some(Visual {
        buf: key,
        anchor: start,
        line,
    });
    let doc = doc(state, buf);
    place(state, buf, &doc, line_no, col)
}

fn swap(state: &mut AppState, buf: Buf) -> Vec<Effect> {
    let doc = doc(state, buf);
    let head = current_anchor(state, buf, &doc);
    let Some(visual) = state.views.visual.as_mut() else {
        return Vec::new();
    };
    let other = std::mem::replace(&mut visual.anchor, head);
    let Some((line, col)) = resolve(state, buf, &other) else {
        return Vec::new();
    };
    place(state, buf, &doc, line, col)
}

fn store_yank(
    state: &mut AppState,
    register: Option<char>,
    text: String,
    linewise: bool,
) -> Vec<Effect> {
    let lines = text.matches('\n').count() + usize::from(!linewise);
    let copied = state
        .registers
        .yank(register, RegisterValue { text, linewise });
    if linewise && lines > 2 {
        state.keys.message(format!("{lines} lines yanked"));
    }
    copied.map(Effect::Copy).into_iter().collect()
}

fn line_block(doc: &Doc, first: usize, last: usize) -> String {
    let mut text = doc.lines[first..=last.min(doc.len() - 1)].join("\n");
    text.push('\n');
    text
}

fn code_block(doc: &Doc, line: usize, around: bool) -> Option<(usize, usize, String)> {
    let gutter = |text: &str| text.trim_start().starts_with('▏');
    if !gutter(&doc.lines[line]) {
        return None;
    }
    let mut first = line;
    while first > 0 && gutter(&doc.lines[first - 1]) {
        first -= 1;
    }
    let mut last = line;
    while last + 1 < doc.len() && gutter(&doc.lines[last + 1]) {
        last += 1;
    }
    let mut code: Vec<String> = doc.lines[first..=last]
        .iter()
        .map(|text| {
            let rest = text.trim_start().trim_start_matches('▏');
            rest.strip_prefix(' ').unwrap_or(rest).to_owned()
        })
        .collect();
    if around {
        code.insert(0, "```".into());
        code.push("```".into());
    }
    let mut text = code.join("\n");
    text.push('\n');
    Some((first, last, text))
}

fn row_lines(state: &AppState, line: usize, around: bool) -> Option<(usize, usize)> {
    let timeline = state.timeline()?;
    let (starts, _) = timeline_starts(state)?;
    let index = starts
        .partition_point(|start| *start <= line)
        .checked_sub(1)?;
    let (width, _) = timeline.viewport_size();
    let height = timeline.item_height(index, width).max(1);
    let gap = usize::from(timeline.items[index].gap);
    let header = timeline.item_header_lines(index);
    let first = starts[index] + gap + if around { 0 } else { header };
    let last = starts[index] + height - 1;
    if first > last {
        return Some((starts[index] + gap.min(height - 1), last));
    }
    Some((first, last))
}

fn yank_target(
    state: &mut AppState,
    buf: Buf,
    target: Target,
    count: Option<usize>,
    register: Option<char>,
) -> Vec<Effect> {
    let doc = doc(state, buf);
    let (line, col) = cursor(state, buf, &doc);
    if let Target::Inner(Object::Quote('`')) | Target::Around(Object::Quote('`')) = target
        && let Some((first, _, text)) = code_block(&doc, line, matches!(target, Target::Around(_)))
    {
        let mut effects = store_yank(state, register, text, true);
        effects.extend(place(state, buf, &doc, first, 0));
        return effects;
    }
    let target = match target {
        Target::Motion(motion) if !has_columns(buf) && is_horizontal(motion) => {
            return Vec::new();
        }
        other => other,
    };
    let pos = doc.pos(line, col);
    let Some(range) = vi::target_range(
        &doc.chars,
        pos,
        target,
        count,
        false,
        &mut state.keys.last_find,
    ) else {
        return Vec::new();
    };
    yank_range(state, buf, &doc, range, register)
}

fn yank_range(
    state: &mut AppState,
    buf: Buf,
    doc: &Doc,
    range: Range,
    register: Option<char>,
) -> Vec<Effect> {
    let (first, start_col) = doc.at(range.start.min(doc.chars.len()));
    let text = if range.linewise {
        let (last, _) = doc.at(range.end.saturating_sub(1).max(range.start));
        let first = if doc.chars.get(range.start) == Some(&'\n') && range.end == doc.chars.len() {
            first + 1
        } else {
            first
        };
        line_block(doc, first.min(last), last)
    } else {
        doc.text(range.start, range.end)
    };
    let mut effects = store_yank(state, register, text, range.linewise);
    let (line, col) = cursor(state, buf, doc);
    let start = if range.linewise {
        (first.min(line), col)
    } else {
        (first, start_col)
    };
    if start < (line, col) {
        effects.extend(place(state, buf, doc, start.0, start.1));
    }
    effects
}

fn yank_row(state: &mut AppState, buf: Buf, around: bool, register: Option<char>) -> Vec<Effect> {
    if buf != Buf::Timeline {
        return Vec::new();
    }
    let doc = doc(state, buf);
    let (line, _) = cursor(state, buf, &doc);
    let Some((first, last)) = row_lines(state, line, around) else {
        return Vec::new();
    };
    let text = line_block(&doc, first, last);
    store_yank(state, register, text, true)
}

fn selection_text(doc: &Doc, selection: Selection) -> String {
    if selection.line {
        return line_block(doc, selection.start.0, selection.end.0);
    }
    let start = doc.pos(selection.start.0, selection.start.1);
    let end = doc.pos(selection.end.0, selection.end.1);
    let end = vi::grapheme_next(&doc.chars, end, 1)
        .max(end + 1)
        .min(doc.chars.len());
    doc.text(start, end)
}

fn visual_yank(state: &mut AppState, buf: Buf, line: bool, register: Option<char>) -> Vec<Effect> {
    let doc = doc(state, buf);
    let Some(mut selection) = selection(state, buf, Some(&doc)) else {
        return Vec::new();
    };
    selection.line |= line;
    let text = selection_text(&doc, selection);
    end_visual(state);
    let mut effects = store_yank(state, register, text, selection.line);
    let col = if selection.line {
        cursor(state, buf, &doc).1
    } else {
        selection.start.1
    };
    effects.extend(place(state, buf, &doc, selection.start.0, col));
    effects
}

fn select_range(state: &mut AppState, buf: Buf, doc: &Doc, start: usize, end: usize, line: bool) {
    let (first, first_col) = doc.at(start);
    let (last, last_col) = doc.at(end.max(start));
    let key = id(state, buf);
    let anchor = anchor(state, buf, first, first_col);
    state.views.visual = Some(Visual {
        buf: key,
        anchor,
        line,
    });
    let _ = place(state, buf, doc, last, last_col);
}

fn visual_object(state: &mut AppState, buf: Buf, target: Target) -> Vec<Effect> {
    let doc = doc(state, buf);
    let (line, col) = cursor(state, buf, &doc);
    if let Target::Inner(Object::Quote('`')) | Target::Around(Object::Quote('`')) = target
        && let Some((first, last, _)) = code_block(&doc, line, false)
    {
        let start = doc.pos(first, 0);
        let end = doc.pos(last, 0);
        select_range(state, buf, &doc, start, end, true);
        return Vec::new();
    }
    let pos = doc.pos(line, col);
    let Some(range) = vi::target_range(
        &doc.chars,
        pos,
        target,
        None,
        false,
        &mut state.keys.last_find,
    ) else {
        return Vec::new();
    };
    if range.end <= range.start {
        return Vec::new();
    }
    let end = if range.linewise {
        range.end.saturating_sub(2).max(range.start)
    } else {
        range.end - 1
    };
    select_range(state, buf, &doc, range.start, end, range.linewise);
    Vec::new()
}

fn visual_row(state: &mut AppState, buf: Buf, around: bool) -> Vec<Effect> {
    if buf != Buf::Timeline {
        return Vec::new();
    }
    let doc = doc(state, buf);
    let (line, _) = cursor(state, buf, &doc);
    let Some((first, last)) = row_lines(state, line, around) else {
        return Vec::new();
    };
    let start = doc.pos(first, 0);
    let end = doc.pos(last, 0);
    select_range(state, buf, &doc, start, end, true);
    Vec::new()
}

fn batch(state: &mut AppState, buf: Buf, action: Batch) -> Vec<Effect> {
    if buf != Buf::Sidebar {
        return Vec::new();
    }
    let Some(selection) = selection(state, buf, None) else {
        return Vec::new();
    };
    end_visual(state);
    let threads: Vec<String> = state
        .rows
        .iter()
        .skip(selection.start.0)
        .take(selection.end.0 + 1 - selection.start.0)
        .filter_map(|row| row.thread().map(|thread| thread.id.clone()))
        .collect();
    if threads.is_empty() {
        return Vec::new();
    }
    use crate::api::Mutation;
    let effects: Vec<Effect> = threads
        .iter()
        .cloned()
        .map(|thread_id| {
            Effect::Mutate(match action {
                Batch::Archive => Mutation::Archive { thread_id },
                Batch::Read => Mutation::MarkRead { thread_id },
                Batch::Pin => Mutation::Pin { thread_id },
            })
        })
        .collect();
    if action == Batch::Archive {
        state.last_archived = threads.last().cloned();
    }
    let verb = match action {
        Batch::Archive => "Archiving",
        Batch::Read => "Marking read",
        Batch::Pin => "Pinning",
    };
    state
        .keys
        .message(format!("{verb} {} threads", threads.len()));
    effects
}

fn open_search(state: &mut AppState, buf: Buf, forward: bool) -> Vec<Effect> {
    let doc = doc(state, buf);
    let (line, col) = cursor(state, buf, &doc);
    let (top, _) = view(state, buf);
    state.mode = Mode::Search(SearchLine {
        input: LineInput::default(),
        forward,
        history: None,
        origin: Origin {
            buf,
            line,
            col,
            top,
        },
    });
    Vec::new()
}

fn restore(state: &mut AppState, origin: Origin) -> Vec<Effect> {
    if focused(state) != Some(origin.buf) {
        return Vec::new();
    }
    let doc = doc(state, origin.buf);
    let mut effects = place(state, origin.buf, &doc, origin.line, origin.col);
    effects.extend(set_top(state, origin.buf, origin.top, doc.len()));
    effects
}

fn report(
    state: &mut AppState,
    doc: &Doc,
    search: &Search,
    at: (usize, usize),
    wrapped: bool,
    forward: bool,
    lines_only: bool,
) {
    if wrapped {
        state.keys.error(if forward {
            "search hit BOTTOM, continuing at TOP"
        } else {
            "search hit TOP, continuing at BOTTOM"
        });
        return;
    }
    let (index, total) = match_index(doc, &search.regex, at.0, at.1, lines_only);
    let prefix = if forward { '/' } else { '?' };
    state
        .keys
        .message(format!("{prefix}{} [{index}/{total}]", search.pattern));
}

fn not_found(state: &mut AppState, pattern: &str) {
    state
        .keys
        .error(format!("E486: Pattern not found: {pattern}"));
}

fn reveal_sidebar_matches(state: &mut AppState) {
    let Some(regex) = state
        .views
        .search
        .as_ref()
        .map(|search| search.regex.clone())
    else {
        return;
    };
    let Some(sidebar) = state.snapshot.sidebar.data.clone() else {
        return;
    };
    let open = std::collections::HashSet::new();
    let rows = crate::sidebar::build_rows(
        &sidebar,
        crate::sidebar::SidebarView {
            collapsed: &open,
            expanded_threads: &state.expanded_threads,
            filter: &state.filter,
        },
    );
    let mut group: Option<&str> = None;
    let mut reveal = Vec::new();
    for row in &rows {
        match row {
            SidebarRow::Group(row) => group = Some(row.key.as_str()),
            SidebarRow::Thread(thread) => {
                if let Some(key) = group
                    && state.collapsed.contains(key)
                    && regex.is_match(&thread.title)
                    && !reveal.contains(&key)
                {
                    reveal.push(key);
                }
            }
        }
    }
    if reveal.is_empty() {
        return;
    }
    for key in reveal {
        state.collapsed.remove(key);
    }
    state.rebuild_rows();
}

fn run_search(
    state: &mut AppState,
    buf: Buf,
    from: (usize, usize),
    forward: bool,
    count: Option<usize>,
) -> Vec<Effect> {
    let Some(search) = state.views.search.clone() else {
        state.keys.error("E35: No previous regular expression");
        return Vec::new();
    };
    let doc = doc(state, buf);
    let mut at = from;
    let mut wrapped = false;
    for _ in 0..count.unwrap_or(1).max(1) {
        if !has_columns(buf) {
            at.1 = if forward { usize::MAX } else { 0 };
        }
        match find_match(
            &doc,
            &search.regex,
            at.0,
            at.1,
            forward,
            state.views.wrapscan,
        ) {
            Some((found, wrap)) => {
                at = found;
                wrapped |= wrap;
            }
            None => {
                not_found(state, &search.pattern);
                return Vec::new();
            }
        }
    }
    if let Some(search) = state.views.search.as_mut() {
        search.highlight = true;
    }
    let current = cursor(state, buf, &doc);
    push_jump(state, buf, current.0, current.1);
    report(
        state,
        &doc,
        &search,
        at,
        wrapped,
        forward,
        !has_columns(buf),
    );
    place(state, buf, &doc, at.0, at.1)
}

fn search_next(state: &mut AppState, buf: Buf, reverse: bool, count: Option<usize>) -> Vec<Effect> {
    let Some(forward) = state
        .views
        .search
        .as_ref()
        .map(|search| search.forward != reverse)
    else {
        state.keys.error("E35: No previous regular expression");
        return Vec::new();
    };
    let doc = doc(state, buf);
    let from = cursor(state, buf, &doc);
    run_search(state, buf, from, forward, count)
}

fn word_at(doc: &Doc, line: usize, col: usize) -> Option<(String, usize)> {
    let chars: Vec<char> = doc.lines[line].chars().collect();
    let keyword = |ch: char| ch.is_alphanumeric() || ch == '_';
    let start_search = (col..chars.len()).find(|index| keyword(chars[*index]))?;
    let mut start = start_search;
    while start > 0 && keyword(chars[start - 1]) {
        start -= 1;
    }
    let mut end = start_search;
    while end < chars.len() && keyword(chars[end]) {
        end += 1;
    }
    Some((chars[start..end].iter().collect(), start))
}

fn set_search(state: &mut AppState, pattern: String, forward: bool) -> bool {
    let Some(regex) = compile(&pattern) else {
        return false;
    };
    if !state
        .views
        .history
        .last()
        .is_some_and(|last| *last == pattern)
    {
        state.views.history.push(pattern.clone());
        if state.views.history.len() > MAX_HISTORY {
            state.views.history.remove(0);
        }
    }
    state.registers.set(
        '/',
        RegisterValue {
            text: pattern.clone(),
            linewise: false,
        },
    );
    state.views.search = Some(Search {
        pattern,
        regex,
        forward,
        highlight: true,
    });
    true
}

fn search_word(
    state: &mut AppState,
    buf: Buf,
    forward: bool,
    whole: bool,
    count: Option<usize>,
) -> Vec<Effect> {
    let doc = doc(state, buf);
    let (line, col) = cursor(state, buf, &doc);
    let Some((word, start)) = word_at(&doc, line, col) else {
        state.keys.error("E348: No string under cursor");
        return Vec::new();
    };
    let escaped = regex::escape(&word);
    let pattern = if whole {
        format!("\\b{escaped}\\b")
    } else {
        escaped
    };
    if !set_search(state, pattern, forward) {
        return Vec::new();
    }
    if buf == Buf::Sidebar {
        reveal_sidebar_matches(state);
    }
    run_search(state, buf, (line, start), forward, count)
}

pub(crate) fn handle_search_key(
    state: &mut AppState,
    mut line: SearchLine,
    key: KeyEvent,
) -> Vec<Effect> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let cancel = |state: &mut AppState, origin: Origin| {
        state.views.preview = None;
        restore(state, origin)
    };
    match key.code {
        KeyCode::Char('g' | 'c') if ctrl => return cancel(state, line.origin),
        KeyCode::Backspace if line.input.text().is_empty() => {
            return cancel(state, line.origin);
        }
        KeyCode::Up | KeyCode::Down => {
            step_history(state, &mut line, key.code == KeyCode::Up);
        }
        KeyCode::Char('p' | 'n') if ctrl => {
            step_history(state, &mut line, key.code == KeyCode::Char('p'));
        }
        _ => match line.input.handle_key(key) {
            InputOutcome::Submit => {
                state.views.preview = None;
                return commit_search(state, line);
            }
            InputOutcome::Cancel => return cancel(state, line.origin),
            _ => line.history = None,
        },
    }
    let effects = preview_search(state, &line);
    state.mode = Mode::Search(line);
    effects
}

fn preview_search(state: &mut AppState, line: &SearchLine) -> Vec<Effect> {
    let origin = line.origin;
    let regex = compile(line.input.text());
    state.views.preview = regex.clone();
    let mut effects = restore(state, origin);
    if let Some(regex) = regex
        && focused(state) == Some(origin.buf)
    {
        let doc = doc(state, origin.buf);
        let col = match (has_columns(origin.buf), line.forward) {
            (true, _) => origin.col,
            (false, true) => usize::MAX,
            (false, false) => 0,
        };
        if let Some((found, _)) = find_match(
            &doc,
            &regex,
            origin.line,
            col,
            line.forward,
            state.views.wrapscan,
        ) {
            effects.extend(place(state, origin.buf, &doc, found.0, found.1));
        }
    }
    effects
}

fn commit_search(state: &mut AppState, line: SearchLine) -> Vec<Effect> {
    let origin = line.origin;
    let mut effects = restore(state, origin);
    let text = line.input.text().to_owned();
    let pattern = if text.is_empty() {
        match state.views.search.as_ref() {
            Some(search) => search.pattern.clone(),
            None => {
                state.keys.error("E35: No previous regular expression");
                return effects;
            }
        }
    } else {
        text
    };
    if !set_search(state, pattern, line.forward) {
        return effects;
    }
    if focused(state) != Some(origin.buf) {
        return effects;
    }
    if origin.buf == Buf::Sidebar {
        reveal_sidebar_matches(state);
        let doc = doc(state, origin.buf);
        let (line_no, col) = cursor(state, origin.buf, &doc);
        return run_search(state, origin.buf, (line_no, col), line.forward, None);
    }
    effects.extend(run_search(
        state,
        origin.buf,
        (origin.line, origin.col),
        line.forward,
        None,
    ));
    effects
}

fn step_history(state: &AppState, line: &mut SearchLine, older: bool) {
    let len = state.views.history.len();
    if len == 0 {
        return;
    }
    let next = match (line.history, older) {
        (None, true) => Some(len - 1),
        (None, false) => None,
        (Some(0), true) => Some(0),
        (Some(index), true) => Some(index - 1),
        (Some(index), false) if index + 1 < len => Some(index + 1),
        (Some(_), false) => None,
    };
    line.history = next;
    line.input = LineInput::new(next.map_or("", |index| state.views.history[index].as_str()));
}

fn jump(state: &mut AppState, buf: Buf, older: bool, count: Option<usize>) -> Vec<Effect> {
    let doc = doc(state, buf);
    let (line, col) = cursor(state, buf, &doc);
    let current = anchor(state, buf, line, col);
    let key = id(state, buf);
    let steps = count.unwrap_or(1).max(1);
    let jumps = state.views.jumps.entry(key).or_default();
    let target = if older {
        if jumps.index >= jumps.list.len() {
            if jumps.list.last() != Some(&current) {
                jumps.list.push(current);
            }
            jumps.index = jumps.list.len() - 1;
        }
        if jumps.index < steps {
            None
        } else {
            jumps.index -= steps;
            jumps.list.get(jumps.index).cloned()
        }
    } else if jumps.index + steps < jumps.list.len() {
        jumps.index += steps;
        jumps.list.get(jumps.index).cloned()
    } else {
        None
    };
    let Some(target) = target else {
        return Vec::new();
    };
    match resolve(state, buf, &target) {
        Some((line, col)) => place(state, buf, &doc, line, col),
        None => Vec::new(),
    }
}

fn set_mark(state: &mut AppState, buf: Buf, name: char) -> Vec<Effect> {
    let doc = doc(state, buf);
    let (line, col) = cursor(state, buf, &doc);
    let mark = anchor(state, buf, line, col);
    match name {
        'a'..='z' | '\'' | '`' => {
            let name = if name == '`' { '\'' } else { name };
            state
                .views
                .marks
                .entry(id(state, buf))
                .or_default()
                .insert(name, mark);
        }
        'A'..='Z' if buf == Buf::Timeline => {
            let thread = state.open_thread.clone().unwrap_or_default();
            state.views.global_marks.insert(name, (thread, mark));
        }
        'A'..='Z' => state
            .keys
            .error("E20: Global marks are only available in the timeline"),
        _ => state
            .keys
            .error("E191: Argument must be a letter or forward/backward quote"),
    }
    Vec::new()
}

fn goto_mark(state: &mut AppState, buf: Buf, name: char, line_only: bool) -> Vec<Effect> {
    if name.is_ascii_uppercase() {
        return goto_global_mark(state, name, line_only);
    }
    let name = if name == '`' { '\'' } else { name };
    let key = id(state, buf);
    let Some(mark) = state
        .views
        .marks
        .get(&key)
        .and_then(|marks| marks.get(&name))
        .cloned()
    else {
        state.keys.error("E20: Mark not set");
        return Vec::new();
    };
    let Some((line, col)) = resolve(state, buf, &mark) else {
        state.keys.error("E20: Mark not set");
        return Vec::new();
    };
    let doc = doc(state, buf);
    let current = cursor(state, buf, &doc);
    push_jump(state, buf, current.0, current.1);
    let col = if line_only {
        doc.first_non_blank(line)
    } else {
        col
    };
    place(state, buf, &doc, line, col)
}

fn goto_global_mark(state: &mut AppState, name: char, line_only: bool) -> Vec<Effect> {
    let Some((thread, mark)) = state.views.global_marks.get(&name).cloned() else {
        state.keys.error("E20: Mark not set");
        return Vec::new();
    };
    let mut effects = Vec::new();
    if state.open_thread.as_deref() != Some(thread.as_str()) {
        note_jump(state);
        effects = state.open(thread.clone());
    }
    state.focus = Focus::Main;
    if let (Some(timeline), Anchor::Row { key, offset, col }) =
        (state.timelines.get_mut(&thread), &mark)
    {
        timeline.selected = Some(key.clone());
        timeline.cursor = Some(LineCursor {
            key: key.clone(),
            offset: *offset,
            col: if line_only { 0 } else { *col },
        });
        timeline.cursor_follow = false;
        timeline.scroll = Scroll::Anchor {
            key: key.clone(),
            offset: offset.saturating_sub(2),
        };
    }
    effects
}

fn yank_to_mark(
    state: &mut AppState,
    buf: Buf,
    name: char,
    line_only: bool,
    register: Option<char>,
) -> Vec<Effect> {
    let name = if name == '`' { '\'' } else { name };
    let key = id(state, buf);
    let Some(mark) = state
        .views
        .marks
        .get(&key)
        .and_then(|marks| marks.get(&name))
        .cloned()
    else {
        state.keys.error("E20: Mark not set");
        return Vec::new();
    };
    let Some((line, col)) = resolve(state, buf, &mark) else {
        state.keys.error("E20: Mark not set");
        return Vec::new();
    };
    let doc = doc(state, buf);
    let current = cursor(state, buf, &doc);
    let pos = doc.pos(current.0, current.1);
    let other = doc.pos(line, col);
    let range = if line_only || !has_columns(buf) {
        vi::lines_between(&doc.chars, pos, other)
    } else {
        Range {
            start: pos.min(other),
            end: pos.max(other),
            linewise: false,
        }
    };
    if !range.linewise && range.end == range.start {
        return Vec::new();
    }
    yank_range(state, buf, &doc, range, register)
}

pub(crate) fn char_act(
    state: &mut AppState,
    buf: Buf,
    act: CharAct,
    ch: char,
    count: Option<usize>,
) -> Vec<Effect> {
    let register = state.keys.register.take();
    match act {
        CharAct::Find { forward, till } => {
            move_cursor(state, buf, Motion::Find { forward, till, ch }, count)
        }
        CharAct::ApplyFind {
            op: Operator::Yank,
            forward,
            till,
        } => yank_target(
            state,
            buf,
            Target::Motion(Motion::Find { forward, till, ch }),
            count,
            register,
        ),
        CharAct::SetMark => set_mark(state, buf, ch),
        CharAct::Mark { line } => goto_mark(state, buf, ch, line),
        CharAct::ApplyMark {
            op: Operator::Yank,
            line,
        } => yank_to_mark(state, buf, ch, line, register),
        _ => {
            state.keys.reset();
            Vec::new()
        }
    }
}

pub(crate) fn takes_char(act: CharAct) -> bool {
    matches!(
        act,
        CharAct::Find { .. }
            | CharAct::ApplyFind {
                op: Operator::Yank,
                ..
            }
            | CharAct::SetMark
            | CharAct::Mark { .. }
            | CharAct::ApplyMark {
                op: Operator::Yank,
                ..
            }
    )
}

pub(crate) fn apply(state: &mut AppState, op: ViewOp, count: Option<usize>) -> Vec<Effect> {
    let register = state.keys.register.take();
    let Some(buf) = focused(state) else {
        return Vec::new();
    };
    match op {
        ViewOp::Move(motion) => move_cursor(state, buf, motion, count),
        ViewOp::Page { down, half } => page(state, buf, down, half, count),
        ViewOp::Scroll(delta) => scroll(state, buf, delta, count),
        ViewOp::Align(position) => align(state, buf, position, count),
        ViewOp::Screen(position) => screen(state, buf, position, count),
        ViewOp::Visual { line } => toggle_visual(state, buf, line),
        ViewOp::Reselect => reselect(state, buf),
        ViewOp::VisualSwap => swap(state, buf),
        ViewOp::Yank(target) => yank_target(state, buf, target, count, register),
        ViewOp::YankRow { around } => yank_row(state, buf, around, register),
        ViewOp::VisualYank { line } => visual_yank(state, buf, line, register),
        ViewOp::VisualObject(target) => visual_object(state, buf, target),
        ViewOp::VisualRow { around } => visual_row(state, buf, around),
        ViewOp::Search { forward } => open_search(state, buf, forward),
        ViewOp::SearchNext { reverse } => search_next(state, buf, reverse, count),
        ViewOp::SearchWord { forward, whole } => search_word(state, buf, forward, whole, count),
        ViewOp::JumpOlder => jump(state, buf, true, count),
        ViewOp::JumpNewer => jump(state, buf, false, count),
        ViewOp::Batch(action) => batch(state, buf, action),
    }
}

pub(crate) fn token_at_cursor(state: &AppState) -> Option<String> {
    let buf = focused(state)?;
    let doc = doc(state, buf);
    let (line, col) = cursor(state, buf, &doc);
    let chars: Vec<char> = doc.lines[line].chars().collect();
    if chars.get(col).is_none_or(|ch| ch.is_whitespace()) {
        return None;
    }
    let mut start = col;
    while start > 0 && !chars[start - 1].is_whitespace() {
        start -= 1;
    }
    let mut end = col;
    while end < chars.len() && !chars[end].is_whitespace() {
        end += 1;
    }
    let token: String = chars[start..end].iter().collect();
    let token = token
        .trim_matches(|ch: char| "()[]{}<>\"'`,;".contains(ch))
        .trim_end_matches(['.', ':'])
        .to_owned();
    (!token.is_empty()).then_some(token)
}

pub(crate) fn marks_listing(state: &AppState) -> Vec<String> {
    let mut lines = vec!["mark line  col text".to_owned()];
    let Some(buf) = focused(state) else {
        return lines;
    };
    let doc = doc(state, buf);
    let mut entries: Vec<(char, usize, usize)> = state
        .views
        .marks
        .get(&id(state, buf))
        .map(|marks| {
            marks
                .iter()
                .filter_map(|(name, mark)| {
                    resolve(state, buf, mark).map(|(line, col)| (*name, line, col))
                })
                .collect()
        })
        .unwrap_or_default();
    entries.sort();
    for (name, line, col) in entries {
        let text: String = doc
            .lines
            .get(line)
            .map(|text| text.trim().chars().take(60).collect())
            .unwrap_or_default();
        lines.push(format!(" {name}  {:>5} {:>4} {text}", line + 1, col));
    }
    let mut globals: Vec<_> = state.views.global_marks.iter().collect();
    globals.sort_by_key(|(name, _)| **name);
    for (name, (thread, _)) in globals {
        lines.push(format!(" {name}  {}", state.thread_title(thread)));
    }
    lines
}

pub(crate) fn delete_marks(state: &mut AppState, args: &str, all: bool) {
    let Some(buf) = focused(state) else {
        return;
    };
    let key = id(state, buf);
    if all {
        if let Some(marks) = state.views.marks.get_mut(&key) {
            marks.retain(|name, _| !name.is_ascii_lowercase());
        }
        return;
    }
    let names: Vec<char> = args.chars().filter(|ch| !ch.is_whitespace()).collect();
    for name in names {
        if name.is_ascii_uppercase() {
            state.views.global_marks.remove(&name);
        } else if let Some(marks) = state.views.marks.get_mut(&key) {
            marks.remove(&name);
        }
    }
}

#[derive(Debug, Clone)]
pub struct Paint {
    pub cursor: Option<(usize, usize)>,
    pub selection: Option<Selection>,
    pub regex: Option<Regex>,
}

impl Paint {
    pub fn cursor_line(&self) -> Option<usize> {
        self.cursor.map(|(line, _)| line)
    }

    pub fn overlay(
        &self,
        buffer: &mut Buffer,
        line: usize,
        text: &str,
        area: Rect,
        hscroll: usize,
        cursor: bool,
    ) {
        if area.width == 0 {
            return;
        }
        let mut xs = Vec::with_capacity(text.len() + 1);
        let mut x = 0;
        xs.push(0);
        for ch in text.chars() {
            x += ch.width().unwrap_or(0);
            xs.push(x);
        }
        let len = xs.len() - 1;
        let width = usize::from(area.width);
        let mut paint = |from: usize, to: usize, style: Style, add: bool| {
            let start = from.saturating_sub(hscroll);
            let end = to.saturating_sub(hscroll).min(width);
            if to <= hscroll || start >= end {
                return;
            }
            let rect = Rect {
                x: area.x + start as u16,
                y: area.y,
                width: (end - start) as u16,
                height: 1,
            };
            if add {
                for column in rect.x..rect.x + rect.width {
                    if let Some(cell) = buffer.cell_mut((column, rect.y)) {
                        let patched = cell.style().add_modifier(Modifier::REVERSED);
                        cell.set_style(patched);
                    }
                }
            } else {
                buffer.set_style(rect, style);
            }
        };
        let cell = |col: usize| -> (usize, usize) {
            let col = col.min(len);
            let from = xs[col];
            let to = if col < len {
                xs[col + 1].max(from + 1)
            } else {
                from + 1
            };
            (from, to)
        };
        if let Some(regex) = &self.regex {
            for (start, end) in line_matches(regex, text) {
                paint(xs[start.min(len)], xs[end.min(len)], SEARCH_STYLE, false);
            }
        }
        if let Some(selection) = self.selection
            && (selection.start.0..=selection.end.0).contains(&line)
        {
            if selection.line {
                paint(0, (xs[len]).max(width + hscroll), VISUAL_STYLE, false);
            } else {
                let first = if line == selection.start.0 {
                    selection.start.1
                } else {
                    0
                };
                let last = if line == selection.end.0 {
                    cell(selection.end.1).1
                } else {
                    xs[len] + 1
                };
                paint(cell(first).0, last, VISUAL_STYLE, false);
            }
        }
        if cursor
            && let Some((cursor_line, col)) = self.cursor
            && cursor_line == line
        {
            let (from, to) = cell(col);
            paint(from, to, Style::new(), true);
        }
    }
}

pub const ROW_SHIFT: u32 = 20;

pub fn row_line(index: usize, offset: usize) -> usize {
    (index << ROW_SHIFT) | offset.min((1 << ROW_SHIFT) - 1)
}

fn timeline_paint(state: &AppState) -> (Option<(usize, usize)>, Option<Selection>) {
    let Some(timeline) = state.timeline() else {
        return (None, None);
    };
    if focused(state) != Some(Buf::Timeline) {
        return (None, None);
    }
    let (width, _) = timeline.viewport_size();
    let locate = |key: &str, offset: usize, matched: bool| -> Option<usize> {
        let index = timeline.resolve_item(key)?;
        let height = timeline.item_height(index, width).max(1);
        let gap = usize::from(timeline.items[index].gap);
        let offset = if matched && timeline.items[index].key == key {
            offset.min(height - 1)
        } else {
            gap
        };
        Some(row_line(index, offset))
    };
    let cursor = timeline.selected.as_deref().and_then(|selected| {
        let (offset, col, matched) = match &timeline.cursor {
            Some(cursor) if cursor.key == selected => (cursor.offset, cursor.col, true),
            _ => (0, 0, false),
        };
        locate(selected, offset, matched).map(|line| (line, col))
    });
    let selection = state.views.visual.as_ref().and_then(|visual| {
        if visual.buf != id(state, Buf::Timeline) {
            return None;
        }
        let Anchor::Row { key, offset, col } = &visual.anchor else {
            return None;
        };
        let start = (locate(key, *offset, true)?, *col);
        let head = cursor?;
        let (start, end) = if start <= head {
            (start, head)
        } else {
            (head, start)
        };
        Some(Selection {
            start,
            end,
            line: visual.line,
        })
    });
    (cursor, selection)
}

pub fn paint(state: &AppState, buf: Buf) -> Paint {
    let active = focused(state) == Some(buf);
    let regex = if matches!(state.mode, Mode::Search(_)) {
        state.views.preview.clone()
    } else {
        state
            .views
            .search
            .as_ref()
            .filter(|search| search.highlight)
            .map(|search| search.regex.clone())
    };
    if buf == Buf::Timeline {
        let (cursor, selection) = timeline_paint(state);
        return Paint {
            cursor,
            selection,
            regex,
        };
    }
    Paint {
        cursor: if active { raw_cursor(state, buf) } else { None },
        selection: if active {
            selection(state, buf, None)
        } else {
            None
        },
        regex,
    }
}

pub fn note_width(state: &AppState, buf: Buf, width: usize) {
    state.views.widths.borrow_mut().insert(buf, width);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(lines: &[&str]) -> Doc {
        Doc::new(lines.iter().map(|line| (*line).to_owned()).collect())
    }

    #[test]
    fn doc_maps_positions_to_lines_and_columns() {
        let doc = doc(&["abc", "", "de"]);
        assert_eq!(doc.pos(2, 1), 6);
        assert_eq!(doc.at(6), (2, 1));
        assert_eq!(doc.at(4), (1, 0));
        assert_eq!(doc.clamp(0, 9), (0, 2));
        assert_eq!(doc.clamp(1, 3), (1, 0));
        assert_eq!(doc.clamp(7, 0), (2, 0));
    }

    #[test]
    fn smart_case_follows_uppercase_and_overrides() {
        assert!(compile("foo").unwrap().is_match("FOO"));
        assert!(!compile("Foo").unwrap().is_match("foo"));
        assert!(compile("Foo\\c").unwrap().is_match("foo"));
        assert!(!compile("\\Cfoo").unwrap().is_match("FOO"));
        assert!(compile("a(b").unwrap().is_match("xa(b"));
        assert!(compile("").is_none());
    }

    #[test]
    fn find_match_wraps_in_both_directions() {
        let doc = doc(&["one foo", "two", "foo three foo"]);
        let regex = compile("foo").unwrap();
        assert_eq!(
            find_match(&doc, &regex, 0, 4, true, true),
            Some(((2, 0), false))
        );
        assert_eq!(
            find_match(&doc, &regex, 2, 10, true, true),
            Some(((0, 4), true))
        );
        assert_eq!(find_match(&doc, &regex, 2, 10, true, false), None);
        assert_eq!(
            find_match(&doc, &regex, 2, 0, false, true),
            Some(((0, 4), false))
        );
        assert_eq!(
            find_match(&doc, &regex, 0, 4, false, true),
            Some(((2, 10), true))
        );
        assert_eq!(match_index(&doc, &regex, 2, 0, false), (2, 3));
        assert_eq!(match_index(&doc, &regex, 2, 0, true), (2, 2));
    }

    #[test]
    fn code_blocks_are_found_by_their_gutter() {
        let doc = doc(&["text", "  ▏ let a = 1;", "  ▏ let b = 2;", "after"]);
        let (first, last, text) = code_block(&doc, 2, false).unwrap();
        assert_eq!((first, last), (1, 2));
        assert_eq!(text, "let a = 1;\nlet b = 2;\n");
        assert_eq!(
            code_block(&doc, 1, true).unwrap().2,
            "```\nlet a = 1;\nlet b = 2;\n```\n"
        );
        assert!(code_block(&doc, 0, false).is_none());
    }

    #[test]
    fn word_under_cursor_skips_to_the_next_keyword() {
        let doc = doc(&["  -- hello_world!"]);
        assert_eq!(word_at(&doc, 0, 0), Some(("hello_world".to_owned(), 5)));
        assert_eq!(word_at(&doc, 0, 8), Some(("hello_world".to_owned(), 5)));
    }
}
