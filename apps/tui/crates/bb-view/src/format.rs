use bb_wire::{JsonObject, JsonValue};

pub fn message_started_at(created_at: i64, started_at: Option<i64>) -> i64 {
    started_at.unwrap_or(created_at)
}

pub fn first_string_field<'a>(record: Option<&'a JsonObject>, keys: &[&str]) -> Option<&'a str> {
    let record = record?;
    keys.iter().find_map(|key| match record.get(*key) {
        Some(JsonValue::String(value)) if !value.is_empty() => Some(value.as_str()),
        _ => None,
    })
}

pub fn capitalize(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

pub fn lower_first(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

pub fn plural(count: usize, singular: &str) -> String {
    plural_with(count, singular, None)
}

pub fn plural_with(count: usize, singular: &str, plural_name: Option<&str>) -> String {
    if count == 1 {
        format!("{count} {singular}")
    } else {
        match plural_name {
            Some(name) => format!("{count} {name}"),
            None => format!("{count} {singular}s"),
        }
    }
}

fn js_round(value: f64) -> f64 {
    (value + 0.5).floor()
}

pub fn duration_to_compact_string(duration_ms: f64) -> String {
    if !duration_ms.is_finite() || duration_ms < 0.0 {
        return "0s".to_owned();
    }
    if duration_ms < 1_000.0 {
        return format!("{}ms", js_round(duration_ms) as i64);
    }
    let total_seconds = js_round(duration_ms / 1_000.0) as i64;
    if total_seconds < 60 {
        return format!("{total_seconds}s");
    }
    format_rounded_duration_seconds(total_seconds)
}

pub fn duration_ms_to_compact_string(duration_ms: i64) -> String {
    duration_to_compact_string(duration_ms as f64)
}

fn format_rounded_duration_seconds(total_seconds: i64) -> String {
    let hours = total_seconds / 3_600;
    let minutes = (total_seconds % 3_600) / 60;
    let seconds = total_seconds % 60;
    let mut parts = Vec::new();
    if hours > 0 {
        parts.push(format!("{hours}h"));
    }
    if minutes > 0 {
        parts.push(format!("{minutes}m"));
    }
    if seconds > 0 {
        parts.push(format!("{seconds}s"));
    }
    parts.join(" ")
}

pub fn message_id(thread_id: &str, kind: &str, key: &str) -> String {
    format!("{thread_id}:{kind}:{key}")
}

pub fn format_diff_count(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

pub fn format_diff_stats_text(added: u64, removed: u64, hide_zero: bool) -> String {
    let added_text = format!("+{}", format_diff_count(added));
    let removed_text = format!("-{}", format_diff_count(removed));
    if !hide_zero {
        return format!("{added_text} {removed_text}");
    }
    let mut parts = Vec::new();
    if added > 0 {
        parts.push(added_text);
    }
    if removed > 0 {
        parts.push(removed_text);
    }
    parts.join(" ")
}

pub fn utf16_len(value: &str) -> usize {
    value.encode_utf16().count()
}

pub fn utf16_prefix(value: &str, units: usize) -> String {
    let mut taken = 0;
    let mut end = 0;
    for (index, ch) in value.char_indices() {
        let width = ch.len_utf16();
        if taken + width > units {
            break;
        }
        taken += width;
        end = index + ch.len_utf8();
    }
    value[..end].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_invalid_durations_as_zero_seconds() {
        assert_eq!(duration_to_compact_string(f64::NAN), "0s");
        assert_eq!(duration_to_compact_string(-1.0), "0s");
    }

    #[test]
    fn formats_sub_second_durations_as_milliseconds() {
        assert_eq!(duration_to_compact_string(0.0), "0ms");
        assert_eq!(duration_to_compact_string(50.0), "50ms");
        assert_eq!(duration_to_compact_string(999.0), "999ms");
    }

    #[test]
    fn rounds_seconds_to_whole_seconds() {
        assert_eq!(duration_to_compact_string(1_499.0), "1s");
        assert_eq!(duration_to_compact_string(1_500.0), "2s");
        assert_eq!(duration_to_compact_string(59_499.0), "59s");
    }

    #[test]
    fn formats_durations_over_a_minute() {
        assert_eq!(duration_to_compact_string(60_000.0), "1m");
        assert_eq!(duration_to_compact_string(89_600.0), "1m 30s");
        assert_eq!(duration_to_compact_string(125_000.0), "2m 5s");
        assert_eq!(duration_ms_to_compact_string(3_661_000), "1h 1m 1s");
    }

    #[test]
    fn pluralizes_labels() {
        assert_eq!(plural(1, "file"), "1 file");
        assert_eq!(plural(2, "file"), "2 files");
        assert_eq!(plural_with(2, "search", Some("searches")), "2 searches");
        assert_eq!(plural(0, "file"), "0 files");
    }

    #[test]
    fn capitalizes_only_the_first_character() {
        assert_eq!(capitalize("hello"), "Hello");
        assert_eq!(capitalize("a"), "A");
        assert_eq!(capitalize(""), "");
        assert_eq!(capitalize("Hello"), "Hello");
    }

    #[test]
    fn returns_the_first_non_empty_string_field() {
        let mut record = JsonObject::new();
        record.insert("first".into(), JsonValue::String(String::new()));
        record.insert("second".into(), JsonValue::String("value".into()));
        record.insert("third".into(), JsonValue::String("ignored".into()));
        assert_eq!(
            first_string_field(Some(&record), &["first", "second", "third"]),
            Some("value")
        );
    }

    #[test]
    fn ignores_missing_empty_and_non_string_fields() {
        let mut record = JsonObject::new();
        record.insert("first".into(), JsonValue::from(1));
        record.insert("second".into(), JsonValue::String(String::new()));
        record.insert("third".into(), JsonValue::Null);
        assert_eq!(
            first_string_field(Some(&record), &["first", "second", "third"]),
            None
        );
        assert_eq!(first_string_field(None, &["first"]), None);
    }

    #[test]
    fn uses_started_at_when_present() {
        assert_eq!(message_started_at(20, Some(10)), 10);
        assert_eq!(message_started_at(20, None), 20);
    }

    #[test]
    fn formats_diff_counts_with_grouping() {
        assert_eq!(format_diff_count(0), "0");
        assert_eq!(format_diff_count(999), "999");
        assert_eq!(format_diff_count(1_000), "1,000");
        assert_eq!(format_diff_count(1_234_567), "1,234,567");
        assert_eq!(format_diff_stats_text(1_200, 3, false), "+1,200 -3");
        assert_eq!(format_diff_stats_text(0, 2, true), "-2");
        assert_eq!(format_diff_stats_text(0, 0, true), "");
    }

    #[test]
    fn slices_by_utf16_units() {
        assert_eq!(utf16_len("a😀"), 3);
        assert_eq!(utf16_prefix("a😀b", 2), "a");
        assert_eq!(utf16_prefix("a😀b", 3), "a😀");
    }
}
