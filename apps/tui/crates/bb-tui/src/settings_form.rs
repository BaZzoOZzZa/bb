use bb_wire::common::{JsonObject, JsonValue};
use bb_wire::plugin::PluginSettingDescriptor;

pub const MANAGED_BRANCH_PREFIX_MAX_LENGTH: usize = 64;

const RESERVED_BRANCH_NAMES: &[&str] = &[
    "AUTO_MERGE",
    "BISECT_HEAD",
    "CHERRY_PICK_HEAD",
    "FETCH_HEAD",
    "HEAD",
    "MERGE_HEAD",
    "ORIG_HEAD",
    "REVERT_HEAD",
];

pub fn is_valid_git_branch_name(name: &str) -> bool {
    let forbidden = |ch: char| {
        ch <= '\u{1f}' || ch == '\u{7f}' || matches!(ch, '\\' | ':' | '~' | '^' | '?' | '*' | '[')
    };
    !name.is_empty()
        && !name.trim().is_empty()
        && !name.starts_with('-')
        && !name.starts_with('/')
        && name != "@"
        && !RESERVED_BRANCH_NAMES.contains(&name)
        && !name.chars().any(forbidden)
        && !name.chars().any(|ch| ch == ' ' || ch == '\t')
        && !name.contains("..")
        && !name.contains("@{")
        && !name.contains("//")
        && !name.ends_with('/')
        && !name.ends_with('.')
        && name.split('/').all(|component| {
            !component.is_empty() && !component.starts_with('.') && !component.ends_with(".lock")
        })
}

pub fn validate_branch_prefix(prefix: &str) -> Result<(), String> {
    if prefix.chars().count() > MANAGED_BRANCH_PREFIX_MAX_LENGTH {
        return Err(format!(
            "Use at most {MANAGED_BRANCH_PREFIX_MAX_LENGTH} characters"
        ));
    }
    if is_valid_git_branch_name(&format!("{prefix}slug-thr_id")) {
        Ok(())
    } else {
        Err("Prefix must start a valid git branch name".into())
    }
}

pub fn validate_server_url(text: &str) -> Result<Option<String>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let url = url::Url::parse(text).map_err(|_| "Enter a full http:// or https:// URL")?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("Use an http:// or https:// URL".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("Leave credentials out of the URL".into());
    }
    Ok(Some(text.to_owned()))
}

pub fn escape_multiline(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out
}

pub fn unescape_multiline(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldKind {
    Boolean,
    Number,
    Select(Vec<String>),
    Project,
    Text { multiline: bool },
    Secret,
    Unsupported,
}

impl FieldKind {
    pub fn of(descriptor: &PluginSettingDescriptor) -> Self {
        match descriptor {
            PluginSettingDescriptor::Boolean { .. } => Self::Boolean,
            PluginSettingDescriptor::Number { .. } => Self::Number,
            PluginSettingDescriptor::Select { options, .. } => Self::Select(options.clone()),
            PluginSettingDescriptor::Project { .. } => Self::Project,
            PluginSettingDescriptor::String {
                secret: Some(true), ..
            } => Self::Secret,
            PluginSettingDescriptor::String { multiline, .. } => Self::Text {
                multiline: *multiline == Some(true),
            },
            PluginSettingDescriptor::Unknown(_) => Self::Unsupported,
        }
    }

    pub fn is_typed(&self) -> bool {
        matches!(self, Self::Number | Self::Text { .. } | Self::Secret)
    }
}

pub fn descriptor_description(descriptor: &PluginSettingDescriptor) -> Option<&str> {
    match descriptor {
        PluginSettingDescriptor::String { description, .. }
        | PluginSettingDescriptor::Boolean { description, .. }
        | PluginSettingDescriptor::Number { description, .. }
        | PluginSettingDescriptor::Select { description, .. }
        | PluginSettingDescriptor::Project { description, .. } => description.as_deref(),
        PluginSettingDescriptor::Unknown(value) => {
            value.get("description").and_then(JsonValue::as_str)
        }
    }
}

pub fn secret_is_set(value: Option<&JsonValue>) -> bool {
    value
        .and_then(|value| value.get("set"))
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
}

pub fn field_display(
    kind: &FieldKind,
    value: Option<&JsonValue>,
    project_name: impl Fn(&str) -> Option<String>,
) -> String {
    match kind {
        FieldKind::Boolean => match value.and_then(JsonValue::as_bool) {
            Some(true) => "on".into(),
            _ => "off".into(),
        },
        FieldKind::Number => match value.and_then(JsonValue::as_f64) {
            Some(number) => format_number(number),
            None => "—".into(),
        },
        FieldKind::Select(_) => match value.and_then(JsonValue::as_str) {
            Some(text) if !text.is_empty() => text.to_owned(),
            _ => "Select…".into(),
        },
        FieldKind::Project => match value.and_then(JsonValue::as_str) {
            Some(id) if !id.is_empty() => project_name(id).unwrap_or_else(|| id.to_owned()),
            _ => "Select a project…".into(),
        },
        FieldKind::Secret => {
            if secret_is_set(value) {
                "[set]".into()
            } else {
                "[not set]".into()
            }
        }
        FieldKind::Text { multiline } => {
            let text = value.and_then(JsonValue::as_str).unwrap_or("");
            if text.is_empty() {
                return "—".into();
            }
            let lines = text.lines().count();
            let first = text.lines().next().unwrap_or("");
            if *multiline && lines > 1 {
                format!("{first} … ({lines} lines)")
            } else {
                first.to_owned()
            }
        }
        FieldKind::Unsupported => value.map_or_else(|| "—".into(), JsonValue::to_string),
    }
}

pub fn format_number(number: f64) -> String {
    if number.fract() == 0.0 && number.abs() < 1e15 {
        format!("{}", number as i64)
    } else {
        number.to_string()
    }
}

pub fn initial_edit_text(kind: &FieldKind, value: Option<&JsonValue>) -> String {
    match kind {
        FieldKind::Number => value
            .and_then(JsonValue::as_f64)
            .map(format_number)
            .unwrap_or_default(),
        FieldKind::Text { multiline } => {
            let text = value.and_then(JsonValue::as_str).unwrap_or("");
            if *multiline {
                escape_multiline(text)
            } else {
                text.to_owned()
            }
        }
        _ => String::new(),
    }
}

pub fn parse_field_input(kind: &FieldKind, text: &str) -> Result<Option<JsonValue>, String> {
    match kind {
        FieldKind::Number => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return Ok(Some(JsonValue::Null));
            }
            match trimmed.parse::<f64>() {
                Ok(number) if number.is_finite() => Ok(serde_json::Number::from_f64(number)
                    .map(|number| Some(JsonValue::Number(number)))
                    .unwrap_or(Some(JsonValue::Null))),
                _ => Err("Enter a finite number".into()),
            }
        }
        FieldKind::Text { multiline: true } => {
            Ok(Some(JsonValue::String(unescape_multiline(text))))
        }
        FieldKind::Text { multiline: false } => Ok(Some(JsonValue::String(text.to_owned()))),
        FieldKind::Secret if text.is_empty() => Ok(None),
        FieldKind::Secret => Ok(Some(JsonValue::String(text.to_owned()))),
        FieldKind::Select(options) => {
            if options.iter().any(|option| option == text) {
                Ok(Some(JsonValue::String(text.to_owned())))
            } else {
                Err(format!("Choose one of: {}", options.join(", ")))
            }
        }
        FieldKind::Project => {
            if text.is_empty() {
                Err("Choose a project".into())
            } else {
                Ok(Some(JsonValue::String(text.to_owned())))
            }
        }
        FieldKind::Boolean => match text {
            "true" | "on" => Ok(Some(JsonValue::Bool(true))),
            "false" | "off" => Ok(Some(JsonValue::Bool(false))),
            _ => Err("Choose on or off".into()),
        },
        FieldKind::Unsupported => Err("This setting can only be changed in the web app".into()),
    }
}

pub fn unchanged(kind: &FieldKind, stored: Option<&JsonValue>, next: &JsonValue) -> bool {
    match (kind, stored) {
        (FieldKind::Secret, _) => false,
        (FieldKind::Number, None) => next.is_null(),
        (FieldKind::Number, Some(stored)) => match (stored.as_f64(), next.as_f64()) {
            (Some(left), Some(right)) => left == right,
            (None, None) => next.is_null() && stored.is_null(),
            _ => false,
        },
        (_, Some(stored)) => stored == next,
        (_, None) => false,
    }
}

pub fn merge_general(general: &JsonValue, key: &str, value: JsonValue) -> Option<JsonObject> {
    let mut object = general.as_object()?.clone();
    object.insert(key.to_owned(), value);
    Some(object)
}

pub fn general_bool(general: &JsonValue, key: &str) -> Option<bool> {
    general.get(key).and_then(JsonValue::as_bool)
}
