use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

use crate::app::AppState;

pub const SPEC: &str = include_str!("../../../docs/keymap-evil.md");

pub const TOPICS: &[&str] = &[
    "states",
    "modeline",
    "buffers",
    "windows",
    "global",
    "leader",
    "motions",
    "counts",
    "search",
    "marks",
    "scrolling",
    "folding",
    "sidebar",
    "timeline",
    "composer",
    "sending",
    "insert",
    "normal",
    "visual",
    "registers",
    "clipboard",
    "minibuffers",
    "ex",
    "set",
    "panel",
    "terminal",
    "notifications",
    "settings",
    "automations",
    "which-key",
    "overrides",
    "mouse",
    "russian",
    "migration",
    "status",
];

fn alias(topic: &str) -> &str {
    match topic {
        "ex" => "Ex commands",
        "set" => "`:set` options",
        "windows" => "Window commands",
        "global" => "Global keys",
        "leader" => "Leader map",
        "insert" => "Insert state",
        "normal" => "Normal state",
        "visual" => "Visual states",
        "overrides" => "Server keybinding overrides",
        "russian" => "Russian layout",
        "status" => "Implementation status",
        "panel" => "Right panel",
        "terminal" => "Terminal tab",
        "notifications" => "Notification center",
        "states" => "States and the modeline",
        other => other,
    }
}

pub fn lines() -> Vec<Vec<Span<'static>>> {
    let mut fence = false;
    SPEC.lines()
        .map(|line| {
            if line.starts_with("```") {
                fence = !fence;
            }
            let style = if line.starts_with('#') {
                Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else if fence || line.starts_with("```") {
                Style::new().fg(Color::Yellow)
            } else if line.starts_with("|---") {
                Style::new().fg(Color::DarkGray)
            } else {
                Style::new()
            };
            vec![Span::styled(line.to_owned(), style)]
        })
        .collect()
}

fn section_range(lines: &[&str], heading: &str) -> std::ops::Range<usize> {
    let start = lines
        .iter()
        .position(|line| line.starts_with("## ") && line.contains(heading))
        .unwrap_or(0);
    let end = lines
        .iter()
        .skip(start + 1)
        .position(|line| line.starts_with("## "))
        .map_or(lines.len(), |offset| start + 1 + offset);
    start..end
}

fn ex_command_line(lines: &[&str], name: &str) -> Option<usize> {
    let full = if name.chars().all(|ch| ch.is_ascii_digit()) || name == "$" {
        None
    } else {
        Some(crate::ex::resolve_name(name)?)
    };
    let range = section_range(lines, "Ex commands");
    lines[range.clone()]
        .iter()
        .position(|line| {
            let Some(cell) = line
                .strip_prefix("| ")
                .and_then(|row| row.split(" | ").next())
            else {
                return false;
            };
            cell.split('`')
                .skip(1)
                .step_by(2)
                .filter_map(|token| token.strip_prefix(':'))
                .any(|token| {
                    let expanded: String = token
                        .chars()
                        .filter(|ch| !matches!(ch, '[' | ']'))
                        .collect();
                    let word: String = expanded
                        .chars()
                        .take_while(|ch| ch.is_ascii_alphabetic())
                        .collect();
                    match full {
                        Some(full) => {
                            !word.is_empty() && crate::ex::resolve_name(&word) == Some(full)
                        }
                        None => expanded.starts_with("{N}") || expanded == "$",
                    }
                })
        })
        .map(|offset| range.start + offset)
}

fn heading_title(line: &str) -> Option<String> {
    let rest = line.strip_prefix('#')?.trim_start_matches('#').trim();
    let rest = match rest.split_once(". ") {
        Some((number, title)) if number.chars().all(|ch| ch.is_ascii_digit()) => title,
        _ => rest,
    };
    Some(rest.to_lowercase())
}

fn heading_line(lines: &[&str], topic: &str) -> Option<usize> {
    if let Ok(number) = topic.parse::<usize>() {
        let prefix = format!("## {number}.");
        return lines.iter().position(|line| line.starts_with(&prefix));
    }
    let lower = alias(topic).to_lowercase();
    let titles: Vec<Option<String>> = lines.iter().map(|line| heading_title(line)).collect();
    titles
        .iter()
        .position(|title| title.as_deref() == Some(lower.as_str()))
        .or_else(|| {
            titles
                .iter()
                .position(|title| title.as_ref().is_some_and(|title| title.contains(&lower)))
        })
}

fn key_line(lines: &[&str], topic: &str) -> Option<usize> {
    let quoted = format!("`{topic}`");
    lines
        .iter()
        .position(|line| {
            line.strip_prefix("| ")
                .and_then(|row| row.split(" | ").next())
                .is_some_and(|cell| cell.contains(&quoted))
        })
        .or_else(|| lines.iter().position(|line| line.contains(&quoted)))
}

fn word_like(topic: &str) -> bool {
    topic.chars().count() >= 3
        && topic
            .chars()
            .all(|ch| ch.is_alphabetic() || ch == '-' || ch == ' ')
}

pub fn find(topic: &str) -> Option<usize> {
    let topic = topic.trim();
    if topic.is_empty() {
        return Some(0);
    }
    let lines: Vec<&str> = SPEC.lines().collect();
    if let Some(name) = topic.strip_prefix(':') {
        let name: String = name
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '$')
            .collect();
        return ex_command_line(&lines, &name);
    }
    let unquoted = topic.trim_matches('\'');
    if unquoted != topic {
        return key_line(&lines, unquoted);
    }
    let anywhere = || {
        let lower = alias(topic).to_lowercase();
        lines
            .iter()
            .position(|line| line.to_lowercase().contains(&lower))
    };
    if TOPICS.contains(&topic) || word_like(topic) || topic.parse::<usize>().is_ok() {
        heading_line(&lines, topic)
            .or_else(anywhere)
            .or_else(|| key_line(&lines, topic))
    } else {
        key_line(&lines, topic)
            .or_else(|| heading_line(&lines, topic))
            .or_else(anywhere)
    }
}

impl AppState {
    pub(crate) fn open_help(&mut self, topic: &str) {
        let Some(line) = find(topic) else {
            self.keys
                .error(format!("E149: Sorry, no help for {}", topic.trim()));
            return;
        };
        self.open_text_pane("Help: keymap-evil.md".to_owned(), lines());
        if let Some(pane) = self.result_pane.as_mut() {
            pane.cursor = line;
            pane.scroll = line;
            pane.column = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line_of(topic: &str) -> &'static str {
        let index = find(topic).unwrap_or_else(|| panic!("no help for {topic}"));
        SPEC.lines().nth(index).expect("line")
    }

    #[test]
    fn topics_land_on_sections_keys_and_ex_commands() {
        assert_eq!(find(""), Some(0));
        assert!(line_of("13").starts_with("## 13. Ex commands"));
        assert!(line_of("ex").starts_with("## 13. Ex commands"));
        assert!(line_of("sidebar").starts_with("## 8. Sidebar"));
        assert!(line_of("which-key").starts_with("## 15. Which-key"));
        assert!(line_of(":q").starts_with("| `:q[uit]`"));
        assert!(line_of(":bn").contains("`:bn[ext]`"));
        assert!(line_of(":sett").contains("`:settings [section]`"));
        assert!(line_of(":5").contains("`:{N}`"));
        assert!(line_of("'hlsearch'").contains("`hlsearch`"));
        assert!(line_of("gx").contains("`gx`"));
        assert!(line_of("search").starts_with("### Search"));
        assert!(line_of("set").contains("`:set` options"));
        assert_eq!(find(":frobnicate"), None);
        assert_eq!(find("zzqqxx"), None);
    }

    #[test]
    fn every_listed_topic_resolves() {
        for topic in TOPICS {
            assert!(find(topic).is_some(), "{topic}");
        }
    }
}
