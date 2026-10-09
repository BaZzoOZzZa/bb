use bb_wire::timeline::WorkflowAgentState;
use bb_wire::{JsonObject, JsonValue};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::format::{utf16_len, utf16_prefix};

pub const LOCAL_WORKFLOW_TASK_TYPE: &str = "local_workflow";
pub const LOCAL_BASH_TASK_TYPE: &str = "local_bash";
pub const LOCAL_AGENT_TASK_TYPE: &str = "local_agent";
pub const LOCAL_SUBAGENT_TASK_TYPE: &str = "local_subagent";

const GENERATED_ID_ALPHABET: &str = "23456789abcdefghijkmnpqrstuvwxyz";
const GENERATED_ID_SUFFIX_LENGTH: usize = 10;

pub fn is_background_command_task_type(task_type: &str) -> bool {
    task_type == LOCAL_BASH_TASK_TYPE
}

pub fn is_background_agent_task_type(task_type: &str) -> bool {
    task_type == LOCAL_AGENT_TASK_TYPE || task_type == LOCAL_SUBAGENT_TASK_TYPE
}

pub fn is_settled_workflow_agent_state(state: WorkflowAgentState) -> bool {
    matches!(
        state,
        WorkflowAgentState::Done | WorkflowAgentState::Failed | WorkflowAgentState::Skipped
    )
}

fn is_glyph_name_char(ch: char) -> bool {
    ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-'
}

pub fn is_namespaced_glyph(glyph: &str) -> bool {
    let Some((plugin, name)) = glyph.split_once('/') else {
        return false;
    };
    !plugin.is_empty()
        && plugin.chars().all(is_glyph_name_char)
        && name
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit())
        && name.chars().all(is_glyph_name_char)
}

pub fn is_raw_thread_id(value: &str) -> bool {
    value.strip_prefix("thr_").is_some_and(|suffix| {
        suffix.len() == GENERATED_ID_SUFFIX_LENGTH
            && suffix.chars().all(|ch| GENERATED_ID_ALPHABET.contains(ch))
    })
}

pub fn format_tool_call_command(tool: &str, args: Option<&JsonObject>) -> String {
    let Some(args) = args.filter(|args| !args.is_empty()) else {
        return tool.to_owned();
    };
    let compact = args
        .iter()
        .map(|(key, value)| {
            let text = match value {
                JsonValue::String(value) => value.trim().to_owned(),
                other => other.to_string(),
            };
            let display = if utf16_len(&text) > 40 {
                format!("{}...", utf16_prefix(&text, 37))
            } else {
                text
            };
            format!("{key}: {display}")
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("{tool} {{ {compact} }}")
}

pub fn display_width(text: &str) -> usize {
    text.graphemes(true)
        .map(|grapheme| {
            if grapheme.is_empty() {
                0
            } else if grapheme.width() >= 2 {
                2
            } else {
                1
            }
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_namespaced_glyphs() {
        assert!(is_namespaced_glyph("my-plugin/icon-1"));
        assert!(!is_namespaced_glyph("Terminal"));
        assert!(!is_namespaced_glyph("Plugin/icon"));
        assert!(!is_namespaced_glyph("plugin/-icon"));
        assert!(!is_namespaced_glyph("plugin/icon/extra"));
    }

    #[test]
    fn recognizes_raw_thread_ids() {
        assert!(is_raw_thread_id("thr_23456789ab"));
        assert!(!is_raw_thread_id("thr_23456789a1"));
        assert!(!is_raw_thread_id("thr_short"));
    }

    #[test]
    fn formats_tool_call_commands() {
        assert_eq!(format_tool_call_command("Tool", None), "Tool");
        let mut args = JsonObject::new();
        assert_eq!(format_tool_call_command("Tool", Some(&args)), "Tool");
        args.insert(
            "query".into(),
            JsonValue::String("  select:TodoWrite ".into()),
        );
        args.insert("limit".into(), JsonValue::from(5));
        args.insert("long".into(), JsonValue::String("x".repeat(50)));
        assert_eq!(
            format_tool_call_command("Tool", Some(&args)),
            format!(
                "Tool {{ limit: 5, long: {}..., query: select:TodoWrite }}",
                "x".repeat(37)
            )
        );
    }

    #[test]
    fn measures_wide_graphemes() {
        assert_eq!(display_width("abc"), 3);
        assert_eq!(display_width("日本"), 4);
        assert_eq!(display_width("😀a"), 3);
        assert_eq!(display_width("e\u{301}"), 1);
    }
}
