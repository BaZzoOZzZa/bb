use crate::app::{AppState, Effect, Mode};
use crate::ex::{ExLine, NAMES, OPTIONS, resolve_name};
use crate::palette::PaletteRequest;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Wild {
    pub start: usize,
    pub typed: String,
    pub items: Vec<String>,
    pub selected: Option<usize>,
    pub pending: Option<(u64, String)>,
    pub note: Option<String>,
}

pub enum Source {
    Items(Vec<String>),
    Files(String),
}

pub struct Context {
    pub start: usize,
    pub token: String,
    pub source: Source,
}

const MERGE: &[&str] = &["squash", "merge", "rebase"];
const PANEL: &[&str] = &["info", "diff", "file", "terminal"];
const TERMINAL: &[&str] = &["new", "attach", "close"];
const SUBMIT_KEYS: &[&str] = &["enter", "ctrl-enter"];
const CLIPBOARD: &[&str] = &["unnamedplus"];

fn ranked(candidates: impl IntoIterator<Item = String>, token: &str) -> Vec<String> {
    let lower = token.to_lowercase();
    let mut prefix = Vec::new();
    let mut inner = Vec::new();
    for candidate in candidates {
        if prefix.contains(&candidate) || inner.contains(&candidate) {
            continue;
        }
        let folded = candidate.to_lowercase();
        if folded.starts_with(&lower) {
            prefix.push(candidate);
        } else if folded.contains(&lower) {
            inner.push(candidate);
        }
    }
    prefix.extend(inner);
    prefix
}

fn words(list: &[&str]) -> Vec<String> {
    list.iter().map(|word| (*word).to_owned()).collect()
}

fn command_names(token: &str) -> Vec<String> {
    NAMES
        .iter()
        .map(|(full, _)| *full)
        .filter(|full| full.starts_with(token))
        .map(str::to_owned)
        .collect()
}

fn option_items(token: &str) -> (usize, Vec<String>) {
    if let Some((name, value)) = token.split_once('=') {
        let values = match name {
            "submitkey" => words(SUBMIT_KEYS),
            "clipboard" => words(CLIPBOARD),
            "organization" | "org" => words(&["project", "chronological", "machine"]),
            "sort" => words(&["updated", "created", "alpha"]),
            "sortdir" => words(&["default", "asc", "desc"]),
            "lifecycle" => words(&["active", "archived", "both"]),
            "envgroup" => words(&["auto", "on", "off"]),
            _ => Vec::new(),
        };
        return (name.len() + 1, ranked(values, value));
    }
    let mut names = vec!["all".to_owned()];
    for option in OPTIONS {
        names.push(option.name.to_owned());
        if option.boolean {
            names.push(format!("no{}", option.name));
            names.push(format!("inv{}", option.name));
        } else {
            names.push(format!("{}=", option.name));
        }
    }
    let names = names
        .into_iter()
        .filter(|name| name.starts_with(token))
        .collect();
    (0, names)
}

impl AppState {
    fn thread_titles(&self) -> Vec<String> {
        self.snapshot
            .sidebar_threads()
            .map(crate::sidebar::thread_title)
            .collect()
    }

    fn project_names(&self) -> Vec<String> {
        self.snapshot
            .sidebar
            .data
            .as_deref()
            .map(|sidebar| {
                sidebar
                    .projects
                    .iter()
                    .map(|project| project.project.name.clone())
                    .filter(|name| !name.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn picker_labels(&self, kind: crate::compose::PickerKind) -> Vec<String> {
        let Some(thread_id) = self.open_thread.as_deref() else {
            return Vec::new();
        };
        let Some(resolved) = self.resolved_execution(thread_id) else {
            return Vec::new();
        };
        match kind {
            crate::compose::PickerKind::Model => resolved
                .model_options
                .iter()
                .map(|option| option.label.clone())
                .collect(),
            crate::compose::PickerKind::Reasoning => resolved
                .reasoning_options
                .iter()
                .map(|option| option.label.clone())
                .collect(),
            crate::compose::PickerKind::Permission => resolved
                .permission_options
                .iter()
                .map(|option| option.label.clone())
                .collect(),
        }
    }

    pub(crate) fn ex_context(&self, head: &str) -> Option<Context> {
        let lead = head.len() - head.trim_start_matches([' ', ':']).len();
        let body = &head[lead..];
        let name_len = body
            .char_indices()
            .find(|(_, ch)| !ch.is_ascii_alphabetic())
            .map_or(body.len(), |(index, _)| index);
        let name = &body[..name_len];
        if name_len == body.len() {
            return Some(Context {
                start: lead,
                token: name.to_owned(),
                source: Source::Items(command_names(name)),
            });
        }
        let full = resolve_name(name)?;
        let rest = &body[name_len..];
        let rest = rest.strip_prefix('!').unwrap_or(rest);
        let args = rest.trim_start();
        let start = head.len() - args.len();
        let token = args.to_owned();
        let items = |list: Vec<String>| Source::Items(ranked(list, args));
        let source = match full {
            "buffer" | "bdelete" | "split" | "vsplit" | "unarchive" | "pin" | "unpin" | "read"
            | "unread" => items(self.thread_titles()),
            "edit" | "find" => Source::Files(token.clone()),
            "project" => items(self.project_names()),
            "settings" => items(
                crate::settings::SettingsSection::ALL
                    .iter()
                    .map(|section| section.label().to_lowercase())
                    .collect(),
            ),
            "set" => {
                let word_start = args.rfind(' ').map_or(0, |index| index + 1);
                let word = &args[word_start..];
                let (offset, names) = option_items(word);
                return Some(Context {
                    start: start + word_start + offset,
                    token: word[offset..].to_owned(),
                    source: Source::Items(names),
                });
            }
            "help" => {
                let mut topics = words(crate::ex_help::TOPICS);
                topics.extend(NAMES.iter().map(|(full, _)| format!(":{full}")));
                if args.starts_with(':') {
                    Source::Items(
                        topics
                            .into_iter()
                            .filter(|topic| topic.starts_with(args))
                            .collect(),
                    )
                } else {
                    items(topics)
                }
            }
            "panel" => items(words(PANEL)),
            "terminal" => items(words(TERMINAL)),
            "automations" => items(words(&["new"])),
            "notifications" => items(words(&["clear"])),
            "merge" => items(words(MERGE)),
            "model" => items(self.picker_labels(crate::compose::PickerKind::Model)),
            "reasoning" => items(self.picker_labels(crate::compose::PickerKind::Reasoning)),
            "permission" => items(self.picker_labels(crate::compose::PickerKind::Permission)),
            _ => return None,
        };
        Some(Context {
            start,
            token,
            source,
        })
    }
}

fn common_prefix(items: &[String]) -> String {
    let Some(first) = items.first() else {
        return String::new();
    };
    let mut end = first.len();
    for item in &items[1..] {
        let mut common = 0;
        for ((index, a), b) in first.char_indices().zip(item.chars()) {
            if a != b {
                break;
            }
            common = index + a.len_utf8();
        }
        end = end.min(common);
    }
    first[..end].to_owned()
}

pub(crate) fn offer(line: &mut ExLine, start: usize, typed: String, items: Vec<String>) {
    match items.len() {
        0 => {
            line.wild = Some(Wild {
                start,
                typed,
                note: Some("No match".into()),
                ..Wild::default()
            });
        }
        1 => {
            line.input.splice(start, &items[0]);
            line.wild = None;
        }
        _ => {
            let prefix = common_prefix(&items);
            let grows = prefix.len() > typed.len()
                && prefix.to_lowercase().starts_with(&typed.to_lowercase());
            let mut wild = Wild {
                start,
                typed: typed.clone(),
                items,
                ..Wild::default()
            };
            if grows {
                line.input.splice(start, &prefix);
                wild.typed = prefix;
            } else {
                wild.selected = Some(0);
                line.input.splice(start, &wild.items[0]);
            }
            line.wild = Some(wild);
        }
    }
}

pub(crate) fn cycle(line: &mut ExLine, forward: bool) -> bool {
    let Some(wild) = line.wild.as_mut() else {
        return false;
    };
    let len = wild.items.len();
    if len == 0 {
        return false;
    }
    wild.selected = match (wild.selected, forward) {
        (None, true) => Some(0),
        (None, false) => Some(len - 1),
        (Some(index), true) if index + 1 < len => Some(index + 1),
        (Some(index), false) if index > 0 => Some(index - 1),
        (Some(_), _) => None,
    };
    let text = wild
        .selected
        .map_or(wild.typed.clone(), |index| wild.items[index].clone());
    let start = wild.start;
    line.input.splice(start, &text);
    true
}

pub(crate) fn complete(state: &mut AppState, line: &mut ExLine, forward: bool) -> Vec<Effect> {
    if line
        .wild
        .as_ref()
        .is_some_and(|wild| !wild.items.is_empty())
    {
        cycle(line, forward);
        return Vec::new();
    }
    let head = line.input.split_at_cursor().0.to_owned();
    let Some(context) = state.ex_context(&head) else {
        line.wild = Some(Wild {
            start: line.input.cursor(),
            note: Some("No completion".into()),
            ..Wild::default()
        });
        return Vec::new();
    };
    match context.source {
        Source::Items(items) => {
            offer(line, context.start, context.token, items);
            Vec::new()
        }
        Source::Files(query) => {
            let note = |text: &str| Wild {
                start: context.start,
                typed: context.token.clone(),
                note: Some(text.to_owned()),
                ..Wild::default()
            };
            if query.trim().is_empty() {
                line.wild = Some(note("Type part of a file name, then TAB"));
                return Vec::new();
            }
            let Some(source) = state.file_source() else {
                line.wild = Some(note("No workspace for file completion"));
                return Vec::new();
            };
            let generation = state.next_palette_generation();
            let mut wild = note("Searching files…");
            wild.pending = Some((generation, query.clone()));
            line.wild = Some(wild);
            vec![Effect::Palette {
                generation,
                request: PaletteRequest::Paths { source, query },
            }]
        }
    }
}

pub(crate) fn files_loaded(
    state: &mut AppState,
    generation: u64,
    query: &str,
    result: Result<Vec<String>, String>,
) -> bool {
    let Mode::Ex(line) = &mut state.mode else {
        return false;
    };
    let Some(wild) = line.wild.as_ref() else {
        return false;
    };
    if wild.pending.as_ref().map(|(g, q)| (*g, q.as_str())) != Some((generation, query)) {
        return false;
    }
    let start = wild.start;
    let typed = wild.typed.clone();
    match result {
        Ok(paths) => offer(line, start, typed, paths),
        Err(error) => {
            line.wild = Some(Wild {
                start,
                typed,
                note: Some(format!("File search failed: {error}")),
                ..Wild::default()
            });
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str) -> ExLine {
        ExLine {
            input: crate::input::LineInput::new(text),
            ..ExLine::default()
        }
    }

    #[test]
    fn common_prefix_respects_char_boundaries() {
        assert_eq!(
            common_prefix(&["settings".into(), "set".into()]),
            "set".to_owned()
        );
        assert_eq!(common_prefix(&["ёлка".into(), "ёж".into()]), "ё".to_owned());
        assert_eq!(common_prefix(&["a".into(), "b".into()]), String::new());
    }

    #[test]
    fn offer_inserts_unique_match_grows_prefix_or_selects_first() {
        let mut ex = line("sett");
        offer(&mut ex, 0, "sett".into(), vec!["settings".into()]);
        assert_eq!(ex.input.text(), "settings");
        assert!(ex.wild.is_none());

        let mut ex = line("b");
        offer(&mut ex, 0, "b".into(), vec!["bnext".into(), "bNext".into()]);
        assert_eq!(ex.input.text(), "bnext");
        assert_eq!(ex.wild.as_ref().and_then(|wild| wild.selected), Some(0));
        cycle(&mut ex, true);
        assert_eq!(ex.input.text(), "bNext");
        cycle(&mut ex, true);
        assert_eq!(ex.input.text(), "b");
        cycle(&mut ex, false);
        assert_eq!(ex.input.text(), "bNext");

        let mut ex = line("set no");
        offer(
            &mut ex,
            4,
            "no".into(),
            vec!["nowrapscan".into(), "nowhichkey".into()],
        );
        assert_eq!(ex.input.text(), "set now");
        assert_eq!(ex.wild.as_ref().and_then(|wild| wild.selected), None);
    }

    #[test]
    fn ranked_puts_prefix_matches_first_and_dedupes() {
        let found = ranked(
            vec![
                "Write docs".into(),
                "Fix docs".into(),
                "docs".into(),
                "docs".into(),
            ],
            "Doc",
        );
        assert_eq!(
            found,
            vec!["docs".to_owned(), "Write docs".into(), "Fix docs".into()]
        );
    }
}
