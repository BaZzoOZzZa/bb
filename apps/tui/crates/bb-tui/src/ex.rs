use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::api::Mutation;
use crate::app::{AppState, Effect, Focus, Mode};
use crate::evil::run_counted;
use crate::ex_complete::Wild;
use crate::input::{InputOutcome, LineInput};
use crate::keymap::Command;
use crate::palette::PaletteKind;
use crate::panel::PanelTab;
use crate::sidebar::SidebarRow;

const MAX_HISTORY: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExLine {
    pub input: LineInput,
    pub history: Option<usize>,
    pub history_prefix: String,
    pub wild: Option<Wild>,
}

pub struct OptionSpec {
    pub name: &'static str,
    pub short: &'static str,
    pub boolean: bool,
}

pub const OPTIONS: &[OptionSpec] = &[
    OptionSpec {
        name: "hlsearch",
        short: "hls",
        boolean: true,
    },
    OptionSpec {
        name: "incsearch",
        short: "is",
        boolean: true,
    },
    OptionSpec {
        name: "ignorecase",
        short: "ic",
        boolean: true,
    },
    OptionSpec {
        name: "smartcase",
        short: "scs",
        boolean: true,
    },
    OptionSpec {
        name: "wrapscan",
        short: "ws",
        boolean: true,
    },
    OptionSpec {
        name: "clipboard",
        short: "cb",
        boolean: false,
    },
    OptionSpec {
        name: "submitkey",
        short: "",
        boolean: false,
    },
    OptionSpec {
        name: "whichkeydelay",
        short: "",
        boolean: false,
    },
    OptionSpec {
        name: "follow",
        short: "",
        boolean: true,
    },
    OptionSpec {
        name: "sidebar",
        short: "",
        boolean: true,
    },
    OptionSpec {
        name: "panel",
        short: "",
        boolean: true,
    },
];

const UNSUPPORTED: &[&str] = &[
    "number",
    "nu",
    "relativenumber",
    "rnu",
    "scrolloff",
    "so",
    "mouse",
];

fn option_spec(name: &str) -> Option<&'static OptionSpec> {
    OPTIONS
        .iter()
        .find(|option| option.name == name || (!option.short.is_empty() && option.short == name))
}

const MUTATING: &[&str] = &[
    "write",
    "wq",
    "xit",
    "wall",
    "bdelete",
    "bundo",
    "unarchive",
    "pin",
    "unpin",
    "read",
    "unread",
    "stop",
    "steer",
    "commit",
    "merge",
    "ready",
    "draft",
];

fn mutates(full: &str, args: &str) -> bool {
    MUTATING.contains(&full)
        || (full == "rename" && !args.is_empty())
        || (full == "terminal" && matches!(args, "new" | "close"))
}

pub const NAMES: &[(&str, usize)] = &[
    ("quit", 1),
    ("qall", 2),
    ("cquit", 2),
    ("write", 1),
    ("wq", 2),
    ("xit", 1),
    ("wall", 2),
    ("edit", 1),
    ("bnext", 2),
    ("bprevious", 2),
    ("bNext", 2),
    ("bdelete", 2),
    ("bundo", 2),
    ("buffer", 1),
    ("buffers", 7),
    ("ls", 2),
    ("files", 5),
    ("unarchive", 5),
    ("split", 2),
    ("vsplit", 2),
    ("new", 3),
    ("vnew", 4),
    ("enew", 3),
    ("close", 3),
    ("only", 2),
    ("rename", 3),
    ("pin", 3),
    ("unpin", 5),
    ("read", 4),
    ("unread", 6),
    ("nohlsearch", 3),
    ("marks", 4),
    ("delmarks", 4),
    ("help", 1),
    ("set", 2),
    ("messages", 3),
    ("registers", 3),
    ("display", 2),
    ("palette", 3),
    ("find", 2),
    ("search", 3),
    ("threads", 7),
    ("project", 4),
    ("filter", 4),
    ("diff", 4),
    ("panel", 5),
    ("sidebar", 4),
    ("terminal", 4),
    ("settings", 4),
    ("automations", 4),
    ("notifications", 5),
    ("attachments", 7),
    ("attach", 6),
    ("model", 5),
    ("reasoning", 3),
    ("permission", 4),
    ("stop", 4),
    ("steer", 5),
    ("queue", 5),
    ("answer", 3),
    ("pending", 4),
    ("commit", 4),
    ("merge", 5),
    ("ready", 5),
    ("draft", 5),
    ("reload", 6),
];

pub fn ex_for(command: Command) -> Option<&'static str> {
    Some(match command {
        Command::Quit => "qa",
        Command::CloseWindowOrQuit => "q",
        Command::SendAndClose => "x",
        Command::Send => "w",
        Command::Steer => "w!",
        Command::Rename => "rename",
        Command::Archive => "bd",
        Command::UndoArchive => "bundo",
        Command::ThreadNext => "bnext",
        Command::ThreadPrevious => "bprevious",
        Command::AlternateThread => "b#",
        Command::PaletteOpen => "palette",
        Command::QuickOpen => "find",
        Command::SearchThreads => "search",
        Command::SwitchProject => "project",
        Command::Filter => "filter",
        Command::DiffToggle => "diff",
        Command::PanelToggle => "panel",
        Command::SidebarToggle => "sidebar",
        Command::TerminalOpen => "terminal",
        Command::TerminalNew => "terminal new",
        Command::TerminalAttach => "terminal attach",
        Command::Settings => "settings",
        Command::Automations => "automations",
        Command::NewAutomation => "automations new",
        Command::Notifications => "notifications",
        Command::ClearNotifications => "notifications clear",
        Command::AttachFile => "attach",
        Command::Attachments => "attachments",
        Command::ModelPicker => "model",
        Command::ReasoningPicker => "reasoning",
        Command::PermissionPicker => "permission",
        Command::StopTurn => "stop",
        Command::QueueList => "queue",
        Command::NextPending => "answer",
        Command::GitCommit => "commit",
        Command::PullRequestMerge => "merge",
        Command::PullRequestReady => "ready",
        Command::PullRequestDraft => "draft",
        Command::Reload => "reload",
        Command::SplitOpen => "vsplit",
        Command::SplitNew => "vnew",
        Command::NewThread => "enew",
        Command::NewThreadInEnvironment => "enew!",
        Command::PaneClose => "close",
        Command::WindowOnly => "only",
        Command::TogglePin => "pin",
        Command::Help => "help",
        Command::Messages => "messages",
        _ => return None,
    })
}

pub fn resolve_name(name: &str) -> Option<&'static str> {
    NAMES
        .iter()
        .find(|(full, min)| name.len() >= *min && full.starts_with(name))
        .map(|(full, _)| *full)
}

impl AppState {
    pub(crate) fn open_ex(&mut self) {
        self.mode = Mode::Ex(ExLine::default());
    }

    fn thread_by_query(&self, query: &str) -> Option<String> {
        let query = query.trim();
        if query.is_empty() {
            return None;
        }
        if let Ok(number) = query.strip_prefix('#').unwrap_or(query).parse::<usize>() {
            return self
                .rows
                .iter()
                .filter_map(SidebarRow::thread)
                .nth(number.checked_sub(1)?)
                .map(|row| row.id.clone());
        }
        let lower = query.to_lowercase();
        let threads: Vec<(String, String)> = self
            .snapshot
            .sidebar_threads()
            .map(|entry| (entry.thread.id.clone(), crate::sidebar::thread_title(entry)))
            .collect();
        threads
            .iter()
            .find(|(id, _)| id == query)
            .or_else(|| {
                threads
                    .iter()
                    .find(|(_, title)| title.to_lowercase() == lower)
            })
            .or_else(|| {
                threads
                    .iter()
                    .find(|(_, title)| title.to_lowercase().contains(&lower))
            })
            .map(|(id, _)| id.clone())
    }

    fn ex_thread(&mut self, args: &str) -> Option<String> {
        if args.trim().is_empty() {
            return self.target_thread();
        }
        let found = self.thread_by_query(args);
        if found.is_none() {
            self.keys
                .error(format!("E94: No matching thread for {}", args.trim()));
        }
        found
    }

    fn run_on_thread(&mut self, args: &str, command: Command) -> Vec<Effect> {
        if args.trim().is_empty() {
            return run_counted(self, command, None);
        }
        let Some(thread_id) = self.ex_thread(args) else {
            return Vec::new();
        };
        self.select_thread_row(&thread_id);
        let focus = self.focus;
        self.focus = Focus::Sidebar;
        let visible = self.sidebar_visible;
        self.sidebar_visible = true;
        let effects = run_counted(self, command, None);
        if self.focus == Focus::Sidebar {
            self.focus = focus;
        }
        self.sidebar_visible = visible;
        effects
    }

    fn option_value(&self, name: &str) -> String {
        let flag = |on: bool| format!("{}{name}", if on { "" } else { "no" });
        match name {
            "hlsearch" => flag(self.views.hlsearch),
            "incsearch" => flag(self.views.incsearch),
            "ignorecase" => flag(self.views.case.ignorecase),
            "smartcase" => flag(self.views.case.smartcase),
            "wrapscan" => flag(self.views.wrapscan),
            "clipboard" => format!(
                "clipboard={}",
                if self.registers.clipboard {
                    "unnamedplus"
                } else {
                    ""
                }
            ),
            "submitkey" => format!(
                "submitkey={}",
                match self.submit_key {
                    crate::composer::SubmitKey::Enter => "enter",
                    crate::composer::SubmitKey::ModifierEnter => "ctrl-enter",
                }
            ),
            "whichkeydelay" => format!("whichkeydelay={}", self.keys.which_key_delay.as_millis()),
            "follow" => flag(
                self.timeline()
                    .is_none_or(|timeline| timeline.scroll == crate::timeline::Scroll::Bottom),
            ),
            "sidebar" => flag(self.sidebar_visible),
            "panel" => flag(self.panel_visible),
            _ => String::new(),
        }
    }

    fn set_option(&mut self, args: &str) -> Vec<Effect> {
        let args = args.trim();
        if args.is_empty() || args == "all" {
            let names: Vec<&str> = if args.is_empty() {
                OPTIONS
                    .iter()
                    .map(|option| option.name)
                    .filter(|name| self.options_set.contains(*name))
                    .collect()
            } else {
                OPTIONS.iter().map(|option| option.name).collect()
            };
            let mut lines = vec![if args.is_empty() {
                "--- Options set this session ---".to_owned()
            } else {
                "--- Options ---".to_owned()
            }];
            lines.extend(
                names
                    .iter()
                    .map(|name| format!("  {}", self.option_value(name))),
            );
            self.show_text_pane("Options", lines);
            return Vec::new();
        }
        let mut effects = Vec::new();
        for word in args.split_whitespace() {
            match self.set_one(word) {
                Ok(more) => effects.extend(more),
                Err(error) => {
                    self.keys.error(error);
                    break;
                }
            }
        }
        effects
    }

    fn set_one(&mut self, word: &str) -> Result<Vec<Effect>, String> {
        if let Some(name) = word.strip_suffix('?') {
            let spec = self.known_option(name, word)?;
            let value = self.option_value(spec.name);
            self.keys.message(format!("  {value}"));
            return Ok(Vec::new());
        }
        if let Some((name, value)) = word.split_once(['=', ':']) {
            let spec = self.known_option(name, word)?;
            if spec.boolean {
                return Err(format!("E474: Invalid argument: {word}"));
            }
            self.options_set.insert(spec.name);
            return self.set_value(spec.name, value, word);
        }
        let (name, flag) = if let Some(name) = word.strip_suffix('!') {
            (name, None)
        } else if let Some(name) = word
            .strip_prefix("inv")
            .filter(|name| option_spec(name).is_some())
        {
            (name, None)
        } else if let Some(name) = word
            .strip_prefix("no")
            .filter(|name| option_spec(name).is_some())
        {
            (name, Some(false))
        } else {
            (word, Some(true))
        };
        let spec = self.known_option(name, word)?;
        if !spec.boolean {
            if flag == Some(true) {
                let value = self.option_value(spec.name);
                self.keys.message(format!("  {value}"));
                return Ok(Vec::new());
            }
            return Err(format!("E474: Invalid argument: {word}"));
        }
        self.options_set.insert(spec.name);
        Ok(self.set_flag(spec.name, flag))
    }

    fn known_option(&self, name: &str, word: &str) -> Result<&'static OptionSpec, String> {
        if UNSUPPORTED.contains(&name) {
            return Err(format!("E519: Option not supported: {word}"));
        }
        option_spec(name).ok_or_else(|| format!("E518: Unknown option: {word}"))
    }

    fn set_value(&mut self, name: &str, value: &str, word: &str) -> Result<Vec<Effect>, String> {
        match name {
            "whichkeydelay" => {
                let ms = value
                    .parse::<u64>()
                    .map_err(|_| format!("E521: Number required after =: {word}"))?;
                self.keys.which_key_delay = Duration::from_millis(ms);
            }
            "clipboard" => {
                self.registers.clipboard = match value {
                    "" => false,
                    "unnamedplus" | "unnamed" => true,
                    _ => return Err(format!("E474: Invalid argument: {word}")),
                };
            }
            "submitkey" => {
                self.submit_key = crate::composer::SubmitKey::parse(value)
                    .ok_or_else(|| format!("E474: Invalid argument: {word}"))?;
            }
            _ => return Err(format!("E474: Invalid argument: {word}")),
        }
        Ok(Vec::new())
    }

    fn set_flag(&mut self, name: &str, flag: Option<bool>) -> Vec<Effect> {
        let toggle = |current: bool| flag.unwrap_or(!current);
        match name {
            "hlsearch" => {
                let on = toggle(self.views.hlsearch);
                self.views.hlsearch = on;
                if let Some(search) = self.views.search.as_mut() {
                    search.highlight = on;
                }
            }
            "incsearch" => self.views.incsearch = toggle(self.views.incsearch),
            "ignorecase" | "smartcase" => {
                if name == "ignorecase" {
                    self.views.case.ignorecase = toggle(self.views.case.ignorecase);
                } else {
                    self.views.case.smartcase = toggle(self.views.case.smartcase);
                }
                let case = self.views.case;
                if let Some(search) = self.views.search.as_mut()
                    && let Some(regex) = crate::rbuf::compile(&search.pattern, case)
                {
                    search.regex = regex;
                }
            }
            "wrapscan" => self.views.wrapscan = toggle(self.views.wrapscan),
            "sidebar" => {
                self.sidebar_visible = toggle(self.sidebar_visible);
                if !self.sidebar_visible && self.focus == Focus::Sidebar {
                    self.focus = Focus::Main;
                }
            }
            "panel" => {
                if toggle(self.panel_visible) != self.panel_visible {
                    return run_counted(self, Command::PanelToggle, None);
                }
            }
            "follow" => {
                let Some(following) = self
                    .timeline()
                    .map(|timeline| timeline.scroll == crate::timeline::Scroll::Bottom)
                else {
                    return Vec::new();
                };
                if toggle(following) != following {
                    return run_counted(self, Command::ToggleFollow, None);
                }
            }
            _ => {}
        }
        Vec::new()
    }

    pub(crate) fn show_text_pane(&mut self, title: &str, lines: Vec<String>) {
        let lines = lines
            .into_iter()
            .map(|line| vec![ratatui::text::Span::raw(line)])
            .collect();
        self.open_text_pane(title.to_owned(), lines);
    }

    fn panel_tab(&mut self, args: &str) -> Vec<Effect> {
        let tab = match args.trim() {
            "" => return run_counted(self, Command::PanelToggle, None),
            "info" => PanelTab::Info,
            "diff" => PanelTab::Diff,
            "file" => PanelTab::File,
            "terminal" | "term" => PanelTab::Terminal,
            other => {
                self.keys.error(format!("E475: Invalid argument: {other}"));
                return Vec::new();
            }
        };
        self.focus = Focus::Panel;
        self.set_panel_tab(tab)
    }

    fn send_all(&mut self) -> Vec<Effect> {
        let threads = self.pane_threads();
        let original = self.open_thread.clone();
        let focus = self.focus;
        let mut effects = Vec::new();
        for thread_id in threads {
            let has_draft = self
                .composers
                .get(&thread_id)
                .is_some_and(|composer| composer.has_input());
            if !has_draft {
                continue;
            }
            if self.open_thread.as_deref() != Some(thread_id.as_str()) {
                effects.extend(self.open(thread_id));
            }
            effects.extend(self.submit(false));
        }
        if let Some(original) = original
            && self.open_thread.as_ref() != Some(&original)
        {
            effects.extend(self.open(original));
        }
        self.focus = focus;
        effects
    }

    fn new_thread_with(&mut self, prompt: &str, environment: bool, split: bool) -> Vec<Effect> {
        let command = if environment {
            Command::NewThreadInEnvironment
        } else {
            Command::NewThread
        };
        let mut effects = run_counted(self, command, None);
        self.split_new_thread = split;
        let now = self.clock;
        if !prompt.trim().is_empty()
            && let Some(form) = self.new_thread.as_mut()
        {
            form.activate_prompt();
            form.composer.insert_text(prompt.trim(), now);
        }
        effects.extend(self.sync_new_thread());
        effects
    }

    fn rename_with(&mut self, title: &str) -> Vec<Effect> {
        let Some(thread_id) = self.target_thread() else {
            return Vec::new();
        };
        let title = title.trim();
        if title.is_empty() {
            return run_counted(self, Command::Rename, None);
        }
        vec![Effect::Mutate(Mutation::Rename {
            thread_id,
            title: title.to_owned(),
        })]
    }

    fn flag_mutation(&mut self, args: &str, build: fn(String) -> Mutation) -> Vec<Effect> {
        match self.ex_thread(args) {
            Some(thread_id) => vec![Effect::Mutate(build(thread_id))],
            None => Vec::new(),
        }
    }

    fn pick_with(&mut self, command: Command, args: &str) -> Vec<Effect> {
        let effects = run_counted(self, command, None);
        if args.is_empty() {
            return effects;
        }
        let Mode::Picker(picker) = &self.mode else {
            return effects;
        };
        let lower = args.to_lowercase();
        let found = picker
            .items
            .iter()
            .position(|item| item.label.to_lowercase() == lower)
            .or_else(|| {
                picker
                    .items
                    .iter()
                    .position(|item| item.label.to_lowercase().starts_with(&lower))
            })
            .or_else(|| {
                picker
                    .items
                    .iter()
                    .position(|item| item.label.to_lowercase().contains(&lower))
            });
        let Some(index) = found else {
            self.keys.error(format!("E475: Invalid argument: {args}"));
            return effects;
        };
        let mut picker = picker.clone();
        picker.selected = index;
        let mut effects = effects;
        effects.extend(crate::compose::handle_picker_key(
            self,
            picker,
            KeyEvent::from(KeyCode::Enter),
        ));
        effects
    }

    fn merge_with(&mut self, args: &str) -> Vec<Effect> {
        let method = match args {
            "" => None,
            method => match ["squash", "merge", "rebase"]
                .iter()
                .position(|candidate| candidate.starts_with(method))
            {
                Some(index) => Some(index),
                None => {
                    self.keys.error(format!("E475: Invalid argument: {method}"));
                    return Vec::new();
                }
            },
        };
        let effects = run_counted(self, Command::PullRequestMerge, None);
        if let Some(method) = method
            && let Some(confirm) = self.panel.confirm.as_mut()
            && confirm.action == crate::panel::GitAction::Merge
        {
            confirm.method = method;
        }
        effects
    }

    pub(crate) fn repeat_ex(&mut self) -> Vec<Effect> {
        match self.registers.get(Some(':')) {
            Some(value) => {
                let line = value.text.clone();
                self.run_ex(&line)
            }
            None => {
                self.keys.error("E30: No previous command line");
                Vec::new()
            }
        }
    }

    pub(crate) fn run_ex(&mut self, line: &str) -> Vec<Effect> {
        let line = line.trim().trim_start_matches(':').trim();
        if line.is_empty() {
            return Vec::new();
        }
        if !self.ex_history.last().is_some_and(|last| last == line) {
            self.ex_history.push(line.to_owned());
            if self.ex_history.len() > MAX_HISTORY {
                self.ex_history.remove(0);
            }
        }
        self.registers.set(
            ':',
            crate::evil::RegisterValue {
                text: line.to_owned(),
                linewise: false,
            },
        );
        self.exec_ex(line)
    }

    pub(crate) fn open_ex_with(&mut self, text: &str) {
        self.mode = Mode::Ex(ExLine {
            input: LineInput::new(text),
            ..ExLine::default()
        });
    }

    pub(crate) fn exec_ex(&mut self, line: &str) -> Vec<Effect> {
        let line = line.trim().trim_start_matches(':').trim();
        if line.is_empty() {
            return Vec::new();
        }
        if let Ok(number) = line.parse::<usize>() {
            return run_counted(self, Command::MoveBottom, Some(number.max(1)));
        }
        if line == "$" {
            return run_counted(self, Command::MoveBottom, None);
        }
        if matches!(line, "b#" | "e#") {
            return run_counted(self, Command::AlternateThread, None);
        }
        let split = line
            .char_indices()
            .find(|(_, ch)| !ch.is_ascii_alphabetic())
            .map_or(line.len(), |(index, _)| index);
        let (name, rest) = line.split_at(split);
        let (bang, args) = match rest.strip_prefix('!') {
            Some(args) => (true, args),
            None => (false, rest),
        };
        let args = args.trim();
        let Some(full) = resolve_name(name) else {
            self.keys
                .error(format!("E492: Not an editor command: {line}"));
            return Vec::new();
        };
        if self.read_only && mutates(full, args) {
            self.keys
                .error(format!("E45: Read-only mode: :{full} would change data"));
            return Vec::new();
        }
        let run = |state: &mut AppState, command| run_counted(state, command, None);
        match full {
            "quit" => run(self, Command::CloseWindowOrQuit),
            "qall" | "cquit" => run(self, Command::Quit),
            "write" if bang => run(self, Command::Steer),
            "write" => run(self, Command::Send),
            "wq" | "xit" => run(self, Command::SendAndClose),
            "wall" => self.send_all(),
            "edit" if args.is_empty() => run(self, Command::Reload),
            "edit" => {
                if self.panel.environment_id.is_none() {
                    self.panel_visible = true;
                    let mut effects = self.sync_panel();
                    effects.extend(self.preview_path(args));
                    return effects;
                }
                self.preview_path(args)
            }
            "bnext" => run(self, Command::ThreadNext),
            "bprevious" | "bNext" => run(self, Command::ThreadPrevious),
            "bdelete" => self.run_on_thread(args, Command::Archive),
            "bundo" => run(self, Command::UndoArchive),
            "buffer" if args.is_empty() => run(self, Command::SearchThreads),
            "buffer" => match self.thread_by_query(args) {
                Some(thread_id) => {
                    let effects = self.open(thread_id);
                    self.focus = Focus::Main;
                    effects
                }
                None => {
                    self.keys
                        .error(format!("E94: No matching thread for {args}"));
                    Vec::new()
                }
            },
            "buffers" | "ls" | "files" => run(self, Command::SearchThreads),
            "unarchive" => self.flag_mutation(args, |thread_id| Mutation::Unarchive { thread_id }),
            "split" | "vsplit" => {
                if !args.is_empty() {
                    match self.thread_by_query(args) {
                        Some(thread_id) => self.select_thread_row(&thread_id),
                        None => {
                            self.keys
                                .error(format!("E94: No matching thread for {args}"));
                            return Vec::new();
                        }
                    }
                }
                run(self, Command::SplitOpen)
            }
            "new" | "vnew" => self.new_thread_with(args, bang, true),
            "enew" => self.new_thread_with(args, bang, false),
            "close" => run(self, Command::PaneClose),
            "only" => run(self, Command::WindowOnly),
            "rename" => self.rename_with(args),
            "pin" => self.flag_mutation(args, |thread_id| Mutation::Pin { thread_id }),
            "unpin" => self.flag_mutation(args, |thread_id| Mutation::Unpin { thread_id }),
            "read" => self.flag_mutation(args, |thread_id| Mutation::MarkRead { thread_id }),
            "unread" => self.flag_mutation(args, |thread_id| Mutation::MarkUnread { thread_id }),
            "nohlsearch" => {
                crate::rbuf::nohlsearch(self);
                Vec::new()
            }
            "marks" => {
                let lines = crate::rbuf::marks_listing(self);
                self.show_text_pane("Marks", lines);
                Vec::new()
            }
            "delmarks" => {
                if !bang && args.is_empty() {
                    self.keys.error("E471: Argument required");
                } else {
                    crate::rbuf::delete_marks(self, args, bang);
                }
                Vec::new()
            }
            "help" => {
                self.open_help(args);
                Vec::new()
            }
            "set" => self.set_option(args),
            "messages" => run(self, Command::Messages),
            "registers" | "display" => {
                let lines = self
                    .registers
                    .listing()
                    .into_iter()
                    .map(|(name, text)| {
                        let preview: String = text.replace('\n', "^J").chars().take(200).collect();
                        format!("\"{name}   {preview}")
                    })
                    .collect();
                self.show_text_pane("Registers", lines);
                Vec::new()
            }
            "palette" => self.open_palette_with(PaletteKind::Commands, args),
            "find" => self.open_palette_with(PaletteKind::Files, args),
            "search" | "threads" => self.open_palette_with(PaletteKind::Threads, args),
            "project" => self.open_palette_with(PaletteKind::Projects, args),
            "filter" => {
                self.sidebar_visible = true;
                self.apply_filter(args);
                Vec::new()
            }
            "diff" => run(self, Command::DiffToggle),
            "panel" => self.panel_tab(args),
            "sidebar" => run(self, Command::SidebarToggle),
            "terminal" => match args {
                "" => run(self, Command::TerminalOpen),
                "new" => run(self, Command::TerminalNew),
                "attach" => run(self, Command::TerminalAttach),
                "close" => {
                    let mut effects = run(self, Command::TerminalOpen);
                    effects.extend(crate::app::legacy_key(
                        self,
                        KeyEvent::from(KeyCode::Char('x')),
                    ));
                    effects
                }
                other => {
                    self.keys.error(format!("E475: Invalid argument: {other}"));
                    Vec::new()
                }
            },
            "settings" => match args {
                "" => run(self, Command::Settings),
                section => match crate::settings::SettingsSection::ALL
                    .iter()
                    .find(|candidate| {
                        candidate
                            .label()
                            .to_lowercase()
                            .starts_with(&section.to_lowercase())
                    }) {
                    Some(section) => run(self, Command::SettingsSection(*section)),
                    None => {
                        self.keys
                            .error(format!("E475: Invalid argument: {section}"));
                        Vec::new()
                    }
                },
            },
            "automations" if args == "new" => run(self, Command::NewAutomation),
            "automations" => run(self, Command::Automations),
            "notifications" if args == "clear" => run(self, Command::ClearNotifications),
            "notifications" => run(self, Command::Notifications),
            "attachments" => run(self, Command::Attachments),
            "attach" => {
                let effects = run(self, Command::AttachFile);
                if !args.is_empty()
                    && let Mode::AttachPrompt { input, .. } = &mut self.mode
                {
                    *input = LineInput::new(args);
                }
                effects
            }
            "model" => self.pick_with(Command::ModelPicker, args),
            "reasoning" => self.pick_with(Command::ReasoningPicker, args),
            "permission" => self.pick_with(Command::PermissionPicker, args),
            "stop" => run(self, Command::StopTurn),
            "steer" => {
                if !args.is_empty()
                    && let Some(thread_id) = self.open_thread.clone()
                    && let Some(composer) = self.composers.get_mut(&thread_id)
                {
                    let now = self.clock;
                    composer.set_draft(crate::prompt::PromptDraft::from_text(args), now);
                }
                run(self, Command::Steer)
            }
            "queue" => run(self, Command::QueueList),
            "answer" | "pending" => run(self, Command::NextPending),
            "commit" => run(self, Command::GitCommit),
            "merge" => self.merge_with(args),
            "ready" => run(self, Command::PullRequestReady),
            "draft" => run(self, Command::PullRequestDraft),
            "reload" => run(self, Command::Reload),
            _ => {
                self.keys
                    .error(format!("E492: Not an editor command: {line}"));
                Vec::new()
            }
        }
    }
}

pub(crate) fn handle_ex_key(state: &mut AppState, mut line: ExLine, key: KeyEvent) -> Vec<Effect> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let wild_items = line
        .wild
        .as_ref()
        .is_some_and(|wild| !wild.items.is_empty());
    let mut effects = Vec::new();
    match key.code {
        KeyCode::Char('g' | 'c') if ctrl => return Vec::new(),
        KeyCode::Backspace if line.input.text().is_empty() => return Vec::new(),
        KeyCode::Tab => effects = crate::ex_complete::complete(state, &mut line, true),
        KeyCode::BackTab => {
            if !crate::ex_complete::cycle(&mut line, false) {
                effects = crate::ex_complete::complete(state, &mut line, false);
            }
        }
        KeyCode::Char('n' | 'p') if ctrl && wild_items => {
            crate::ex_complete::cycle(&mut line, key.code == KeyCode::Char('n'));
        }
        KeyCode::Up | KeyCode::Down => {
            line.wild = None;
            step_history(state, &mut line, key.code == KeyCode::Up);
        }
        KeyCode::Char('p' | 'n') if ctrl => {
            line.wild = None;
            step_history(state, &mut line, key.code == KeyCode::Char('p'));
        }
        _ => {
            line.wild = None;
            match line.input.handle_key(key) {
                InputOutcome::Submit => {
                    let text = line.input.text().to_owned();
                    return state.run_ex(&text);
                }
                InputOutcome::Cancel => return Vec::new(),
                _ => line.history = None,
            }
        }
    }
    state.mode = Mode::Ex(line);
    effects
}

fn step_history(state: &AppState, line: &mut ExLine, older: bool) {
    if line.history.is_none() {
        if !older {
            return;
        }
        line.history_prefix = line.input.text().to_owned();
    }
    let prefix = line.history_prefix.as_str();
    let matches = |index: &usize| state.ex_history[*index].starts_with(prefix);
    let len = state.ex_history.len();
    let next = match (line.history, older) {
        (None, _) => (0..len).rev().find(matches),
        (Some(index), true) => (0..index).rev().find(matches).or(Some(index)),
        (Some(index), false) => (index + 1..len).find(matches),
    };
    if next.is_none() && line.history.is_none() {
        return;
    }
    line.history = next;
    line.input = LineInput::new(next.map_or(prefix, |index| state.ex_history[index].as_str()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_resolve_by_shortest_abbreviation() {
        assert_eq!(resolve_name("q"), Some("quit"));
        assert_eq!(resolve_name("qa"), Some("qall"));
        assert_eq!(resolve_name("w"), Some("write"));
        assert_eq!(resolve_name("bn"), Some("bnext"));
        assert_eq!(resolve_name("vs"), Some("vsplit"));
        assert_eq!(resolve_name("se"), Some("set"));
        assert_eq!(resolve_name("sea"), Some("search"));
        assert_eq!(resolve_name("ren"), Some("rename"));
        assert_eq!(resolve_name("re"), None);
        assert_eq!(resolve_name("zzz"), None);
    }
}
