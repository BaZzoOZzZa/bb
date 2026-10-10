use std::collections::HashSet;

use serde_json::json;

use crate::common::JsonValue;

pub const THREAD_LIST_PLUGIN_ID: &str = "thread-list";
pub const PREFERENCES_CHANNEL: &str = "preferences";
pub const LIST_PREFERENCES_METHOD: &str = "listPreferences";
pub const SET_PREFERENCE_METHOD: &str = "setPreference";

const STRING_MAX_LENGTH: usize = 1_024;
const LIST_MAX_LENGTH: usize = 10_000;

macro_rules! pref_enum {
    ($name:ident { $($variant:ident => $value:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name {
            $($variant),+
        }

        impl $name {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $value),+
                }
            }

            pub fn parse(value: &str) -> Option<Self> {
                match value {
                    $($value => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

pref_enum!(OrganizationMode {
    Project => "project",
    Chronological => "chronological",
    Machine => "machine",
});

pref_enum!(ChronologicalSort {
    Updated => "updated",
    Created => "created",
    Alpha => "alpha",
    None => "none",
});

pref_enum!(SortDirection {
    Default => "default",
    Ascending => "ascending",
    Descending => "descending",
});

pref_enum!(ThreadLifecycle {
    Active => "active",
    Archived => "archived",
});

pref_enum!(PreferenceKey {
    ShowProviderIcons => "showProviderIcons",
    ThreadLifecycles => "threadLifecycles",
    OrganizationMode => "organizationMode",
    EnvironmentGrouping => "environmentGrouping",
    GroupByReadStatus => "groupByReadStatus",
    ChronologicalSort => "chronologicalSort",
    SortDirection => "sortDirection",
    SectionOrder => "sectionOrder",
    ManualSectionOrder => "manualSectionOrder",
    MachineSectionOrder => "machineSectionOrder",
    HiddenGroups => "hiddenGroups",
    RowActions => "rowActions",
    CollapsedSections => "collapsedSections",
    CollapsedProjects => "collapsedProjects",
    CollapsedThreads => "collapsedThreads",
    CollapsedEnvironments => "collapsedEnvironments",
    CollapsedThreadSections => "collapsedThreadSections",
    CollapsedMachines => "collapsedMachines",
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum EnvironmentGrouping {
    #[default]
    Auto,
    On,
    Off,
}

impl EnvironmentGrouping {
    pub fn to_json(self) -> JsonValue {
        match self {
            Self::Auto => json!("auto"),
            Self::On => json!(true),
            Self::Off => json!(false),
        }
    }

    fn from_json(value: &JsonValue) -> Option<Self> {
        match value {
            JsonValue::String(text) if text == "auto" => Some(Self::Auto),
            JsonValue::Bool(true) => Some(Self::On),
            JsonValue::Bool(false) => Some(Self::Off),
            _ => None,
        }
    }
}

const THREAD_ROW_ACTION_IDS: &[&str] = &[
    "split", "copyLink", "read", "pin", "move", "rename", "archive",
];
const THREAD_ROW_ACTION_LIMIT: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadListPreferences {
    pub show_provider_icons: bool,
    pub thread_lifecycles: Vec<ThreadLifecycle>,
    pub organization_mode: OrganizationMode,
    pub environment_grouping: EnvironmentGrouping,
    pub group_by_read_status: bool,
    pub chronological_sort: ChronologicalSort,
    pub sort_direction: SortDirection,
    pub section_order: Vec<String>,
    pub manual_section_order: Vec<String>,
    pub machine_section_order: Vec<String>,
    pub hidden_groups: Vec<String>,
    pub row_actions: Vec<String>,
    pub collapsed_sections: Vec<String>,
    pub collapsed_projects: Vec<String>,
    pub collapsed_threads: Vec<String>,
    pub collapsed_environments: Vec<String>,
    pub collapsed_thread_sections: Vec<String>,
    pub collapsed_machines: Vec<String>,
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

impl Default for ThreadListPreferences {
    fn default() -> Self {
        Self {
            show_provider_icons: true,
            thread_lifecycles: vec![ThreadLifecycle::Active],
            organization_mode: OrganizationMode::Chronological,
            environment_grouping: EnvironmentGrouping::Auto,
            group_by_read_status: false,
            chronological_sort: ChronologicalSort::Updated,
            sort_direction: SortDirection::Default,
            section_order: strings(&["pinned", "projects", "threads"]),
            manual_section_order: strings(&["pinned", "sections", "threads"]),
            machine_section_order: strings(&["pinned", "machines", "threads"]),
            hidden_groups: Vec::new(),
            row_actions: strings(&["archive"]),
            collapsed_sections: Vec::new(),
            collapsed_projects: Vec::new(),
            collapsed_threads: Vec::new(),
            collapsed_environments: Vec::new(),
            collapsed_thread_sections: Vec::new(),
            collapsed_machines: Vec::new(),
        }
    }
}

fn parse_string_list(value: &JsonValue, item: impl Fn(&str) -> bool) -> Option<Vec<String>> {
    let items = value.as_array()?;
    if items.len() > LIST_MAX_LENGTH {
        return None;
    }
    items
        .iter()
        .map(|entry| {
            let text = entry.as_str()?;
            let length = text.encode_utf16().count();
            ((1..=STRING_MAX_LENGTH).contains(&length) && item(text)).then(|| text.to_owned())
        })
        .collect()
}

fn dedupe(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

fn is_hidden_group_id(value: &str) -> bool {
    if value == "threads" {
        return true;
    }
    ["project:", "section:", "machine:"].iter().any(|prefix| {
        value
            .strip_prefix(prefix)
            .is_some_and(|rest| !rest.is_empty() && !rest.chars().any(char::is_whitespace))
    })
}

fn parse_enum<T>(value: &JsonValue, parse: fn(&str) -> Option<T>) -> Option<T> {
    value.as_str().and_then(parse)
}

fn parse_lifecycles(value: &JsonValue) -> Option<Vec<ThreadLifecycle>> {
    let items = value.as_array()?;
    if items.is_empty() || items.len() > 2 {
        return None;
    }
    let parsed: Option<Vec<ThreadLifecycle>> = items
        .iter()
        .map(|item| parse_enum(item, ThreadLifecycle::parse))
        .collect();
    let parsed = parsed?;
    let unique: HashSet<ThreadLifecycle> = parsed.iter().copied().collect();
    (unique.len() == parsed.len()).then_some(parsed)
}

fn parse_row_actions(value: &JsonValue) -> Option<Vec<String>> {
    let items = value.as_array()?;
    let known: Vec<String> = items
        .iter()
        .filter_map(JsonValue::as_str)
        .filter(|id| THREAD_ROW_ACTION_IDS.contains(id))
        .map(str::to_owned)
        .collect();
    let mut known = dedupe(known);
    known.truncate(THREAD_ROW_ACTION_LIMIT);
    Some(known)
}

impl ThreadListPreferences {
    pub fn from_list_response(value: &JsonValue) -> Self {
        let mut preferences = Self::default();
        if let Some(values) = value.get("preferences").and_then(JsonValue::as_object) {
            for key in PreferenceKey::ALL {
                if let Some(value) = values.get(key.as_str()) {
                    preferences.apply(*key, value);
                }
            }
        }
        preferences
    }

    pub fn apply(&mut self, key: PreferenceKey, value: &JsonValue) -> bool {
        let list = |item: fn(&str) -> bool| parse_string_list(value, item);
        let any = |_: &str| true;
        match key {
            PreferenceKey::ShowProviderIcons => {
                value.as_bool().map(|v| self.show_provider_icons = v)
            }
            PreferenceKey::ThreadLifecycles => {
                parse_lifecycles(value).map(|v| self.thread_lifecycles = v)
            }
            PreferenceKey::OrganizationMode => {
                parse_enum(value, OrganizationMode::parse).map(|v| self.organization_mode = v)
            }
            PreferenceKey::EnvironmentGrouping => {
                EnvironmentGrouping::from_json(value).map(|v| self.environment_grouping = v)
            }
            PreferenceKey::GroupByReadStatus => {
                value.as_bool().map(|v| self.group_by_read_status = v)
            }
            PreferenceKey::ChronologicalSort => {
                parse_enum(value, ChronologicalSort::parse).map(|v| self.chronological_sort = v)
            }
            PreferenceKey::SortDirection => {
                parse_enum(value, SortDirection::parse).map(|v| self.sort_direction = v)
            }
            PreferenceKey::SectionOrder => list(any).map(|v| self.section_order = v),
            PreferenceKey::ManualSectionOrder => list(any).map(|v| self.manual_section_order = v),
            PreferenceKey::MachineSectionOrder => list(any).map(|v| self.machine_section_order = v),
            PreferenceKey::HiddenGroups => {
                list(is_hidden_group_id).map(|v| self.hidden_groups = dedupe(v))
            }
            PreferenceKey::RowActions => parse_row_actions(value).map(|v| self.row_actions = v),
            PreferenceKey::CollapsedSections => list(|item| matches!(item, "pinned" | "threads"))
                .map(|v| self.collapsed_sections = v),
            PreferenceKey::CollapsedProjects => list(any).map(|v| self.collapsed_projects = v),
            PreferenceKey::CollapsedThreads => list(any).map(|v| self.collapsed_threads = v),
            PreferenceKey::CollapsedEnvironments => {
                list(any).map(|v| self.collapsed_environments = v)
            }
            PreferenceKey::CollapsedThreadSections => {
                list(any).map(|v| self.collapsed_thread_sections = v)
            }
            PreferenceKey::CollapsedMachines => list(any).map(|v| self.collapsed_machines = v),
        }
        .is_some()
    }

    pub fn value(&self, key: PreferenceKey) -> JsonValue {
        match key {
            PreferenceKey::ShowProviderIcons => json!(self.show_provider_icons),
            PreferenceKey::ThreadLifecycles => json!(
                self.thread_lifecycles
                    .iter()
                    .map(|lifecycle| lifecycle.as_str())
                    .collect::<Vec<_>>()
            ),
            PreferenceKey::OrganizationMode => json!(self.organization_mode.as_str()),
            PreferenceKey::EnvironmentGrouping => self.environment_grouping.to_json(),
            PreferenceKey::GroupByReadStatus => json!(self.group_by_read_status),
            PreferenceKey::ChronologicalSort => json!(self.chronological_sort.as_str()),
            PreferenceKey::SortDirection => json!(self.sort_direction.as_str()),
            PreferenceKey::SectionOrder => json!(self.section_order),
            PreferenceKey::ManualSectionOrder => json!(self.manual_section_order),
            PreferenceKey::MachineSectionOrder => json!(self.machine_section_order),
            PreferenceKey::HiddenGroups => json!(self.hidden_groups),
            PreferenceKey::RowActions => json!(self.row_actions),
            PreferenceKey::CollapsedSections => json!(self.collapsed_sections),
            PreferenceKey::CollapsedProjects => json!(self.collapsed_projects),
            PreferenceKey::CollapsedThreads => json!(self.collapsed_threads),
            PreferenceKey::CollapsedEnvironments => json!(self.collapsed_environments),
            PreferenceKey::CollapsedThreadSections => json!(self.collapsed_thread_sections),
            PreferenceKey::CollapsedMachines => json!(self.collapsed_machines),
        }
    }

    pub fn shows_active(&self) -> bool {
        self.thread_lifecycles.is_empty()
            || self.thread_lifecycles.contains(&ThreadLifecycle::Active)
    }

    pub fn shows_archived(&self) -> bool {
        self.thread_lifecycles.contains(&ThreadLifecycle::Archived)
    }

    pub fn groups_by_environment(&self) -> bool {
        if self.group_by_read_status {
            return false;
        }
        match self.environment_grouping {
            EnvironmentGrouping::Auto => self.organization_mode != OrganizationMode::Chronological,
            EnvironmentGrouping::On => true,
            EnvironmentGrouping::Off => false,
        }
    }
}

pub fn set_preference_input(key: PreferenceKey, value: JsonValue) -> JsonValue {
    json!({ "key": key.as_str(), "value": value })
}

pub fn parse_preference_signal(payload: &JsonValue) -> Option<(PreferenceKey, JsonValue)> {
    let object = payload.as_object()?;
    if object.len() != 2 {
        return None;
    }
    let key = PreferenceKey::parse(object.get("key")?.as_str()?)?;
    let value = object.get("value")?.clone();
    Some((key, value))
}

pub fn parse_set_preference_reply(reply: &JsonValue) -> Option<(PreferenceKey, JsonValue)> {
    let key = PreferenceKey::parse(reply.get("key")?.as_str()?)?;
    Some((key, reply.get("value")?.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_response_parses_each_key_and_falls_back_per_key() {
        let response = json!({"preferences": {
            "organizationMode": "machine",
            "threadLifecycles": ["active", "archived"],
            "environmentGrouping": false,
            "chronologicalSort": "alpha",
            "sortDirection": "ascending",
            "sectionOrder": ["threads", "project:p1"],
            "hiddenGroups": ["project:p1", "project:p1", "machine:h1"],
            "collapsedSections": ["pinned"],
            "collapsedProjects": ["p2"],
            "collapsedMachines": "nope",
            "rowActions": ["pin", "bogus", "pin", "read", "move", "rename"],
            "showProviderIcons": false,
            "groupByReadStatus": true
        }});
        let preferences = ThreadListPreferences::from_list_response(&response);
        assert_eq!(preferences.organization_mode, OrganizationMode::Machine);
        assert!(preferences.shows_active() && preferences.shows_archived());
        assert_eq!(preferences.environment_grouping, EnvironmentGrouping::Off);
        assert_eq!(preferences.chronological_sort, ChronologicalSort::Alpha);
        assert_eq!(preferences.sort_direction, SortDirection::Ascending);
        assert_eq!(preferences.section_order, ["threads", "project:p1"]);
        assert_eq!(preferences.hidden_groups, ["project:p1", "machine:h1"]);
        assert_eq!(preferences.collapsed_sections, ["pinned"]);
        assert_eq!(preferences.collapsed_projects, ["p2"]);
        assert!(preferences.collapsed_machines.is_empty());
        assert_eq!(preferences.row_actions, ["pin", "read", "move"]);
        assert!(!preferences.show_provider_icons);
        assert!(preferences.group_by_read_status);
        assert_eq!(
            preferences.manual_section_order,
            ["pinned", "sections", "threads"]
        );
    }

    #[test]
    fn invalid_values_are_rejected_whole() {
        let mut preferences = ThreadListPreferences::default();
        assert!(!preferences.apply(PreferenceKey::OrganizationMode, &json!("tree")));
        assert!(!preferences.apply(PreferenceKey::ThreadLifecycles, &json!([])));
        assert!(!preferences.apply(
            PreferenceKey::ThreadLifecycles,
            &json!(["active", "active"])
        ));
        assert!(!preferences.apply(PreferenceKey::HiddenGroups, &json!(["project:"])));
        assert!(!preferences.apply(PreferenceKey::HiddenGroups, &json!(["projects"])));
        assert!(!preferences.apply(PreferenceKey::HiddenGroups, &json!(["project:a b"])));
        assert!(!preferences.apply(PreferenceKey::CollapsedSections, &json!(["projects"])));
        assert!(!preferences.apply(PreferenceKey::SectionOrder, &json!(["ok", ""])));
        assert!(!preferences.apply(PreferenceKey::EnvironmentGrouping, &json!("on")));
        assert_eq!(preferences, ThreadListPreferences::default());
        assert!(preferences.apply(PreferenceKey::EnvironmentGrouping, &json!(true)));
        assert_eq!(
            preferences.value(PreferenceKey::EnvironmentGrouping),
            json!(true)
        );
    }

    #[test]
    fn environment_grouping_auto_follows_mode_and_read_status() {
        let mut preferences = ThreadListPreferences::default();
        assert!(!preferences.groups_by_environment());
        preferences.organization_mode = OrganizationMode::Project;
        assert!(preferences.groups_by_environment());
        preferences.group_by_read_status = true;
        assert!(!preferences.groups_by_environment());
        preferences.group_by_read_status = false;
        preferences.environment_grouping = EnvironmentGrouping::Off;
        assert!(!preferences.groups_by_environment());
    }

    #[test]
    fn signals_are_strict_and_values_roundtrip() {
        assert_eq!(
            parse_preference_signal(&json!({"key": "organizationMode", "value": "project"})),
            Some((PreferenceKey::OrganizationMode, json!("project")))
        );
        assert_eq!(
            parse_preference_signal(&json!({"key": "organizationMode"})),
            None
        );
        assert_eq!(
            parse_preference_signal(&json!({"key": "nope", "value": 1})),
            None
        );
        assert_eq!(
            parse_preference_signal(&json!({"key": "sortDirection", "value": "x", "extra": 1})),
            None
        );
        let preferences = ThreadListPreferences::default();
        for key in PreferenceKey::ALL {
            let mut copy = ThreadListPreferences::default();
            assert!(copy.apply(*key, &preferences.value(*key)), "{key}");
        }
        assert_eq!(
            set_preference_input(PreferenceKey::SortDirection, json!("descending")),
            json!({"key": "sortDirection", "value": "descending"})
        );
    }
}
