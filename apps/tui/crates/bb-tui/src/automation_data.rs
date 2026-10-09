use std::collections::BTreeMap;

use bb_wire::common::JsonValue;
use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveDateTime, TimeZone, Timelike, Utc};
use serde::Deserialize;

pub const PLUGIN_ID: &str = "automations";
pub const SIGNAL_CHANNEL: &str = "automations";
pub const PERSONAL_PROJECT_ID: &str = "proj_personal";
pub const NAME_MAX_LENGTH: usize = 200;
pub const CRON_MAX_LENGTH: usize = 100;
const CRON_FIELD_COUNT: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(
    tag = "triggerType",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Trigger {
    Schedule { cron: String, timezone: String },
    Once { run_at: i64 },
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(
    tag = "mode",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Execution {
    Agent {
        prompt: String,
        provider_id: String,
        model: String,
        #[serde(default)]
        reasoning_level: String,
        #[serde(default)]
        service_tier: Option<String>,
        #[serde(default)]
        permission_mode: String,
        #[serde(default)]
        environment: JsonValue,
        #[serde(default)]
        target_thread_id: Option<String>,
    },
    Script {
        #[serde(default)]
        script: Option<String>,
        #[serde(default)]
        script_file: Option<String>,
        #[serde(default)]
        interpreter: Option<String>,
        #[serde(default)]
        timeout_ms: Option<u64>,
        #[serde(default)]
        env: Option<BTreeMap<String, String>>,
        #[serde(default)]
        working_directory: Option<JsonValue>,
        #[serde(default)]
        stored_script_path: Option<String>,
        #[serde(default)]
        resolved_working_directory: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    Running,
    Succeeded,
    Failed,
    Skipped,
    #[serde(other)]
    Unknown,
}

impl RunStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "Running",
            Self::Succeeded => "Succeeded",
            Self::Failed => "Failed",
            Self::Skipped => "Skipped",
            Self::Unknown => "Unknown",
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Self::Running => "◐",
            Self::Succeeded => "✓",
            Self::Failed => "✗",
            Self::Skipped => "↷",
            Self::Unknown => "?",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Automation {
    pub id: String,
    pub project_id: String,
    pub name: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, deserialize_with = "bb_wire::common::lenient_option")]
    pub trigger: Option<Trigger>,
    #[serde(default, deserialize_with = "bb_wire::common::lenient_option")]
    pub execution: Option<Execution>,
    #[serde(default)]
    pub origin: String,
    #[serde(default)]
    pub created_by_thread_id: Option<String>,
    #[serde(default)]
    pub next_run_at: Option<i64>,
    #[serde(default)]
    pub last_run_at: Option<i64>,
    #[serde(default)]
    pub run_count: u64,
    #[serde(default, deserialize_with = "bb_wire::common::lenient_option")]
    pub last_run_status: Option<RunStatus>,
    #[serde(default)]
    pub last_run_thread_id: Option<String>,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default)]
    pub problem: Option<String>,
}

impl Automation {
    pub fn route(&self) -> Route {
        Route {
            project_id: self.project_id.clone(),
            automation_id: self.id.clone(),
        }
    }

    pub fn is_unreadable(&self) -> bool {
        self.problem.as_deref() == Some("invalid-stored-data")
            || self.trigger.is_none()
            || self.execution.is_none()
    }

    pub fn needs_prompt(&self) -> bool {
        self.problem.as_deref() == Some("missing-agent-prompt")
            || matches!(&self.execution, Some(Execution::Agent { prompt, .. }) if prompt.is_empty())
    }

    pub fn lifecycle(&self, now_ms: i64) -> Option<OneShotLifecycle> {
        one_shot_lifecycle(
            self.enabled,
            self.trigger.as_ref()?,
            self.run_count,
            self.last_run_status,
            now_ms,
        )
    }

    pub fn allows_toggle(&self, now_ms: i64) -> bool {
        !self.is_unreadable()
            && matches!(
                self.lifecycle(now_ms),
                None | Some(OneShotLifecycle::Scheduled | OneShotLifecycle::Paused)
            )
    }

    pub fn status_label(&self, now_ms: i64) -> String {
        if self.is_unreadable() {
            return "Invalid stored data".into();
        }
        schedule_status_label(
            self.enabled,
            self.next_run_at,
            self.trigger.as_ref(),
            self.run_count,
            self.last_run_status,
            now_ms,
        )
    }

    pub fn cadence(&self) -> String {
        match &self.trigger {
            Some(trigger) => format_trigger(trigger),
            None => "Unknown schedule".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Route {
    pub project_id: String,
    pub automation_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct OverviewProject {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct OverviewEntry {
    pub automation: Automation,
    pub project: OverviewProject,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct OverviewResponse {
    pub automations: Vec<OverviewEntry>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub id: String,
    #[serde(default)]
    pub automation_id: String,
    #[serde(default)]
    pub run_mode: String,
    #[serde(default)]
    pub thread_id: Option<String>,
    pub status: RunStatus,
    #[serde(default)]
    pub trigger: String,
    #[serde(default)]
    pub skip_reason: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub output: Option<String>,
    #[serde(default)]
    pub exit_code: Option<i64>,
    #[serde(default)]
    pub scheduled_for: i64,
    #[serde(default)]
    pub started_at: i64,
    #[serde(default)]
    pub finished_at: Option<i64>,
}

impl Run {
    pub fn duration(&self) -> Option<String> {
        let finished = self.finished_at?;
        let seconds = (finished - self.started_at) as f64 / 1000.0;
        if seconds < 0.0 {
            return None;
        }
        Some(if seconds < 10.0 {
            format!("{seconds:.1}s")
        } else {
            format!("{seconds:.0}s")
        })
    }

    pub fn openable_thread(&self) -> Option<&str> {
        if self.run_mode == "script" {
            return None;
        }
        self.thread_id.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunPage {
    pub runs: Vec<Run>,
    #[serde(default)]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RunReply {
    pub run: Run,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OneShotLifecycle {
    Scheduled,
    Paused,
    Expired,
    Running,
    Completed,
    Failed,
    Skipped,
}

pub fn one_shot_lifecycle(
    enabled: bool,
    trigger: &Trigger,
    run_count: u64,
    last_run_status: Option<RunStatus>,
    now_ms: i64,
) -> Option<OneShotLifecycle> {
    let Trigger::Once { run_at } = trigger else {
        return None;
    };
    if enabled {
        return Some(OneShotLifecycle::Scheduled);
    }
    if run_count > 0 {
        return Some(match last_run_status {
            Some(RunStatus::Running) => OneShotLifecycle::Running,
            Some(RunStatus::Failed) => OneShotLifecycle::Failed,
            Some(RunStatus::Skipped) => OneShotLifecycle::Skipped,
            _ => OneShotLifecycle::Completed,
        });
    }
    Some(if *run_at <= now_ms {
        OneShotLifecycle::Expired
    } else {
        OneShotLifecycle::Paused
    })
}

pub fn schedule_status_label(
    enabled: bool,
    next_run_at: Option<i64>,
    trigger: Option<&Trigger>,
    run_count: u64,
    last_run_status: Option<RunStatus>,
    now_ms: i64,
) -> String {
    if let Some(trigger) = trigger {
        match one_shot_lifecycle(enabled, trigger, run_count, last_run_status, now_ms) {
            Some(OneShotLifecycle::Running) => return "Running".into(),
            Some(OneShotLifecycle::Failed) => return "Failed".into(),
            Some(OneShotLifecycle::Skipped) => return "Skipped".into(),
            Some(OneShotLifecycle::Completed) => return "Completed".into(),
            Some(OneShotLifecycle::Expired) => return "Expired — edit to reschedule".into(),
            _ => {}
        }
    }
    if !enabled {
        return "Paused".into();
    }
    match next_run_at {
        None => "Not scheduled".into(),
        Some(at) => format!("Next {}", format_run_time(at)),
    }
}

pub fn format_run_time(timestamp_ms: i64) -> String {
    format_run_time_in(timestamp_ms, &Local)
}

pub fn format_run_time_in<Tz: TimeZone>(timestamp_ms: i64, tz: &Tz) -> String {
    let Some(utc) = DateTime::<Utc>::from_timestamp_millis(timestamp_ms) else {
        return "—".into();
    };
    let local = utc.with_timezone(tz);
    let hour = local.hour();
    let hour12 = match hour % 12 {
        0 => 12,
        other => other,
    };
    let meridiem = if hour < 12 { "AM" } else { "PM" };
    format!(
        "{} {}, {}:{:02} {}",
        MONTHS[local.month0() as usize],
        local.day(),
        hour12,
        local.minute(),
        meridiem
    )
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const MONTH_NAMES: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

pub fn format_trigger(trigger: &Trigger) -> String {
    match trigger {
        Trigger::Once { .. } => "One time".into(),
        Trigger::Schedule { cron, .. } => describe_cron(cron),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FieldKind {
    Minute,
    Hour,
    DayOfMonth,
    Month,
    DayOfWeek,
}

impl FieldKind {
    const ALL: [FieldKind; 5] = [
        FieldKind::Minute,
        FieldKind::Hour,
        FieldKind::DayOfMonth,
        FieldKind::Month,
        FieldKind::DayOfWeek,
    ];

    fn bounds(self) -> (u32, u32) {
        match self {
            Self::Minute => (0, 59),
            Self::Hour => (0, 23),
            Self::DayOfMonth => (1, 31),
            Self::Month => (1, 12),
            Self::DayOfWeek => (0, 7),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Minute => "minute",
            Self::Hour => "hour",
            Self::DayOfMonth => "day-of-month",
            Self::Month => "month",
            Self::DayOfWeek => "day-of-week",
        }
    }

    fn name_value(self, text: &str) -> Option<u32> {
        let lower = text.to_ascii_lowercase();
        let names: &[&str] = match self {
            Self::Month => &[
                "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
            ],
            Self::DayOfWeek => &["sun", "mon", "tue", "wed", "thu", "fri", "sat"],
            _ => return None,
        };
        let index = names.iter().position(|name| *name == lower)? as u32;
        Some(if self == Self::Month {
            index + 1
        } else {
            index
        })
    }

    fn value(self, text: &str) -> Result<u32, String> {
        let value = match text.parse::<u32>() {
            Ok(value) => value,
            Err(_) => self
                .name_value(text)
                .ok_or_else(|| format!("Invalid {} value \"{text}\"", self.label()))?,
        };
        let (min, max) = self.bounds();
        if value < min || value > max {
            return Err(format!(
                "{} value {value} is outside {min}-{max}",
                capitalize(self.label())
            ));
        }
        Ok(value)
    }
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn validate_field(kind: FieldKind, text: &str) -> Result<(), String> {
    for part in text.split(',') {
        if part.is_empty() {
            return Err(format!("Empty {} list item", kind.label()));
        }
        let (base, step) = match part.split_once('/') {
            Some((base, step)) => (base, Some(step)),
            None => (part, None),
        };
        if let Some(step) = step {
            match step.parse::<u32>() {
                Ok(step) if step > 0 => {}
                _ => return Err(format!("Invalid {} step \"{step}\"", kind.label())),
            }
        }
        if base == "*" {
            continue;
        }
        match base.split_once('-') {
            Some((start, end)) => {
                let start = kind.value(start)?;
                let end = kind.value(end)?;
                if start > end {
                    return Err(format!(
                        "{} range {start}-{end} is reversed",
                        capitalize(kind.label())
                    ));
                }
            }
            None => {
                kind.value(base)?;
            }
        }
    }
    Ok(())
}

pub fn validate_cron(cron: &str) -> Result<(), String> {
    if cron.len() > CRON_MAX_LENGTH {
        return Err(format!(
            "Cron expression must be at most {CRON_MAX_LENGTH} characters"
        ));
    }
    let fields: Vec<&str> = cron.split_whitespace().collect();
    if fields.len() != CRON_FIELD_COUNT {
        return Err("Cron expression must have exactly 5 fields (minute hour day-of-month month day-of-week)".into());
    }
    for (kind, field) in FieldKind::ALL.iter().zip(fields) {
        validate_field(*kind, field)?;
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Spec {
    Any,
    Every(u32),
    Range(u32, u32),
    List(Vec<u32>),
}

fn spec(kind: FieldKind, text: &str) -> Option<Spec> {
    if text == "*" {
        return Some(Spec::Any);
    }
    if let Some(step) = text.strip_prefix("*/") {
        return step.parse().ok().map(Spec::Every);
    }
    if !text.contains(',')
        && let Some((start, end)) = text.split_once('-')
    {
        return Some(Spec::Range(kind.value(start).ok()?, kind.value(end).ok()?));
    }
    text.split(',')
        .map(|part| kind.value(part).ok())
        .collect::<Option<Vec<_>>>()
        .map(Spec::List)
}

fn clock(hour: u32, minute: u32) -> String {
    let hour12 = match hour % 12 {
        0 => 12,
        other => other,
    };
    let meridiem = if hour < 12 { "AM" } else { "PM" };
    if minute == 0 {
        format!("{hour12}{meridiem}")
    } else {
        format!("{hour12}:{minute:02}{meridiem}")
    }
}

fn describe_time(minute: &Spec, hour: &Spec) -> Option<String> {
    Some(match (minute, hour) {
        (Spec::Any, Spec::Any) => "Every minute".into(),
        (Spec::Every(step), Spec::Any) => format!("Every {step} min"),
        (Spec::List(minutes), Spec::Any) if minutes == &[0] => "Every hour".into(),
        (Spec::List(minutes), Spec::Any) if minutes.len() == 1 => {
            format!("{} min past the hour", minutes[0])
        }
        (Spec::Range(start, end), Spec::Any) => format!("Minutes {start}-{end} past the hour"),
        (Spec::List(minutes), Spec::List(hours)) if minutes.len() == 1 => hours
            .iter()
            .map(|hour| clock(*hour, minutes[0]))
            .collect::<Vec<_>>()
            .join(", "),
        (Spec::Any, Spec::Range(start, end)) => {
            format!("Every minute, {}-{}", clock(*start, 0), clock(*end, 59))
        }
        (Spec::Every(step), Spec::Range(start, end)) => {
            format!("Every {step} min, {}-{}", clock(*start, 0), clock(*end, 59))
        }
        (Spec::Every(step), Spec::List(hours)) if hours.len() == 1 => format!(
            "Every {step} min, {}-{}",
            clock(hours[0], 0),
            clock(hours[0], 59)
        ),
        (Spec::List(minutes), Spec::Range(start, end)) if minutes == &[0] => {
            format!("Every hour, {}-{}", clock(*start, 0), clock(*end, 0))
        }
        (Spec::List(minutes), Spec::Range(start, end)) if minutes.len() == 1 => format!(
            "{} min past the hour, {}-{}",
            minutes[0],
            clock(*start, 0),
            clock(*end, 59)
        ),
        _ => return None,
    })
}

fn day_name(day: u32) -> &'static str {
    DAYS[(day % 7) as usize]
}

fn describe_days(dow: &Spec) -> Option<Option<String>> {
    Some(match dow {
        Spec::Any => None,
        Spec::Range(start, end) => Some(format!("{}-{}", day_name(*start), day_name(*end))),
        Spec::List(days) => Some(
            days.iter()
                .map(|day| day_name(*day))
                .collect::<Vec<_>>()
                .join(", "),
        ),
        Spec::Every(_) => return None,
    })
}

fn describe_dom(dom: &Spec) -> Option<Option<String>> {
    Some(match dom {
        Spec::Any => None,
        Spec::List(days) if days.len() == 1 => Some(format!("on day {} of the month", days[0])),
        Spec::Range(start, end) => Some(format!("between day {start} and {end} of the month")),
        Spec::Every(step) => Some(format!("every {step} days")),
        Spec::List(_) => return None,
    })
}

fn describe_month(month: &Spec) -> Option<Option<String>> {
    Some(match month {
        Spec::Any => None,
        Spec::List(months) => Some(format!(
            "only in {}",
            months
                .iter()
                .map(|month| MONTH_NAMES[(*month as usize).saturating_sub(1).min(11)])
                .collect::<Vec<_>>()
                .join(", ")
        )),
        Spec::Range(start, end) => Some(format!(
            "{}-{}",
            MONTH_NAMES[(*start as usize).saturating_sub(1).min(11)],
            MONTH_NAMES[(*end as usize).saturating_sub(1).min(11)]
        )),
        Spec::Every(step) => Some(format!("every {step} months")),
    })
}

fn append(text: &mut String, part: String) {
    if text.ends_with("AM") || text.ends_with("PM") {
        text.push(' ');
    } else {
        text.push_str(", ");
    }
    text.push_str(&part);
}

pub fn describe_cron(cron: &str) -> String {
    if validate_cron(cron).is_err() {
        return "Custom schedule".into();
    }
    let fields: Vec<&str> = cron.split_whitespace().collect();
    let described = (|| {
        let minute = spec(FieldKind::Minute, fields[0])?;
        let hour = spec(FieldKind::Hour, fields[1])?;
        let dom = spec(FieldKind::DayOfMonth, fields[2])?;
        let month = spec(FieldKind::Month, fields[3])?;
        let dow = spec(FieldKind::DayOfWeek, fields[4])?;
        let mut text = describe_time(&minute, &hour)?;
        for part in [
            describe_dom(&dom)?,
            describe_days(&dow)?,
            describe_month(&month)?,
        ]
        .into_iter()
        .flatten()
        {
            append(&mut text, part);
        }
        Some(text)
    })();
    described.unwrap_or_else(|| cron.split_whitespace().collect::<Vec<_>>().join(" "))
}

fn parse_duration_ms(text: &str) -> Option<i64> {
    let text = text.trim();
    let split = text.find(|ch: char| !ch.is_ascii_digit())?;
    let (number, unit) = text.split_at(split);
    let number: i64 = number.parse().ok()?;
    let unit_ms = match unit.trim() {
        "s" | "sec" | "secs" | "second" | "seconds" => 1_000,
        "m" | "min" | "mins" | "minute" | "minutes" => 60_000,
        "h" | "hr" | "hrs" | "hour" | "hours" => 3_600_000,
        "d" | "day" | "days" => 86_400_000,
        _ => return None,
    };
    number.checked_mul(unit_ms)
}

pub fn parse_run_at(text: &str, now_ms: i64) -> Result<i64, String> {
    parse_run_at_in(text, now_ms, &Local)
}

pub fn parse_run_at_in<Tz: TimeZone>(text: &str, now_ms: i64, tz: &Tz) -> Result<i64, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("Enter a run time".into());
    }
    let relative = trimmed
        .strip_prefix("in ")
        .or_else(|| trimmed.strip_prefix('+'))
        .unwrap_or(trimmed);
    let at = if let Some(delay) = parse_duration_ms(relative) {
        now_ms + delay
    } else if let Ok(parsed) = DateTime::parse_from_rfc3339(trimmed) {
        parsed.timestamp_millis()
    } else {
        let normalized = trimmed.replacen('T', " ", 1);
        let naive = ["%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M"]
            .iter()
            .find_map(|format| NaiveDateTime::parse_from_str(&normalized, format).ok())
            .or_else(|| {
                NaiveDate::parse_from_str(&normalized, "%Y-%m-%d")
                    .ok()
                    .and_then(|date| date.and_hms_opt(9, 0, 0))
            })
            .ok_or_else(|| {
                "Use YYYY-MM-DD HH:MM, an ISO time, or a delay like 30m, 2h, 1d".to_owned()
            })?;
        tz.from_local_datetime(&naive)
            .earliest()
            .ok_or_else(|| "That local time does not exist".to_owned())?
            .timestamp_millis()
    };
    if at <= now_ms {
        return Err("One-shot run time must be in the future".into());
    }
    Ok(at)
}

pub fn format_run_at_input(timestamp_ms: i64) -> String {
    DateTime::<Utc>::from_timestamp_millis(timestamp_ms)
        .map(|utc| {
            utc.with_timezone(&Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

pub fn local_timezone() -> String {
    if let Ok(tz) = std::env::var("TZ") {
        let tz = tz.trim_start_matches(':');
        if tz == "UTC" || tz.contains('/') && !tz.starts_with('/') {
            return tz.to_owned();
        }
    }
    std::fs::read_link("/etc/localtime")
        .ok()
        .and_then(|path| {
            let text = path.to_string_lossy().into_owned();
            text.split_once("zoneinfo/")
                .map(|(_, zone)| zone.to_owned())
        })
        .filter(|zone| !zone.is_empty())
        .unwrap_or_else(|| "UTC".into())
}

pub fn environment_label(environment: &JsonValue, target_thread: Option<&str>) -> String {
    if target_thread.is_some() {
        return "Existing thread".into();
    }
    match environment.get("type").and_then(JsonValue::as_str) {
        Some("reuse") => match environment.get("environmentId").and_then(JsonValue::as_str) {
            Some(id) => format!("Reuse environment {id}"),
            None => "Reuse environment".into(),
        },
        Some("project-default") => "Project default".into(),
        Some("host") => {
            let workspace = environment.get("workspace");
            match workspace
                .and_then(|workspace| workspace.get("type"))
                .and_then(JsonValue::as_str)
            {
                Some("managed-worktree") => {
                    match workspace
                        .and_then(|workspace| workspace.get("baseBranch"))
                        .and_then(|base| base.get("name"))
                        .and_then(JsonValue::as_str)
                    {
                        Some(branch) => format!("New worktree from {branch}"),
                        None => "New worktree".into(),
                    }
                }
                Some("personal") => "Personal workspace".into(),
                _ => workspace
                    .and_then(|workspace| workspace.get("path"))
                    .and_then(JsonValue::as_str)
                    .map_or_else(|| "Workspace".into(), str::to_owned),
            }
        }
        _ => "Unknown environment".into(),
    }
}

pub fn working_directory_label(
    working_directory: Option<&JsonValue>,
    resolved: Option<&str>,
) -> String {
    let kind = working_directory
        .and_then(|value| value.get("type"))
        .and_then(JsonValue::as_str);
    let base = match kind {
        Some("project") => "Project checkout".to_owned(),
        Some("path") => working_directory
            .and_then(|value| value.get("path"))
            .and_then(JsonValue::as_str)
            .unwrap_or("Path")
            .to_owned(),
        _ => "Automation storage".to_owned(),
    };
    match resolved {
        Some(path) if kind != Some("path") => format!("{base} ({path})"),
        _ => base,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cron_cadence_matches_the_web_formatter() {
        for (cron, expected) in [
            ("*/5 9-17 * * *", "Every 5 min, 9AM-5:59PM"),
            ("0 9-17 * * 1-5", "Every hour, 9AM-5PM Mon-Fri"),
            ("15 9-17 * * *", "15 min past the hour, 9AM-5:59PM"),
            ("0 0-2 * * *", "Every hour, 12AM-2AM"),
            ("0 9,17 * * 1,3,5", "9AM, 5PM Mon, Wed, Fri"),
            ("30 9,12,17 * * *", "9:30AM, 12:30PM, 5:30PM"),
            ("10-20 * * * *", "Minutes 10-20 past the hour"),
            ("*/5 * * * *", "Every 5 min"),
            ("0 9 * * 1-5", "9AM Mon-Fri"),
            ("invalid", "Custom schedule"),
            ("0 9 * * MON-FRI", "9AM Mon-Fri"),
            ("0 8 1 * *", "8AM on day 1 of the month"),
            ("*/10 * * * 0", "Every 10 min, Sun"),
            ("5,10 */2 * * *", "5,10 */2 * * *"),
        ] {
            assert_eq!(describe_cron(cron), expected, "{cron}");
        }
    }

    #[test]
    fn cron_validation_reports_the_failing_field() {
        assert!(validate_cron("0 9 * * 1-5").is_ok());
        assert!(validate_cron("*/15 0-23/2 1,15 JAN-jun sun").is_ok());
        assert!(
            validate_cron("0 9 * *")
                .unwrap_err()
                .contains("exactly 5 fields")
        );
        assert!(
            validate_cron("60 9 * * *")
                .unwrap_err()
                .contains("Minute value 60")
        );
        assert!(
            validate_cron("0 9 * * 5-1")
                .unwrap_err()
                .contains("reversed")
        );
        assert!(validate_cron("*/0 * * * *").unwrap_err().contains("step"));
        assert!(validate_cron("0 9 * foo *").unwrap_err().contains("month"));
    }

    #[test]
    fn run_at_accepts_delays_local_times_and_iso() {
        let now = 1_800_000_000_000;
        assert_eq!(parse_run_at_in("30m", now, &Utc), Ok(now + 30 * 60_000));
        assert_eq!(parse_run_at_in("in 2h", now, &Utc), Ok(now + 7_200_000));
        assert_eq!(parse_run_at_in("+1d", now, &Utc), Ok(now + 86_400_000));
        let at = Utc.with_ymd_and_hms(2027, 3, 4, 9, 30, 0).unwrap();
        assert_eq!(
            parse_run_at_in("2027-03-04 09:30", now, &Utc),
            Ok(at.timestamp_millis())
        );
        assert_eq!(
            parse_run_at_in("2027-03-04T10:30:00+01:00", now, &Utc),
            Ok(at.timestamp_millis())
        );
        assert!(
            parse_run_at_in("2020-01-01 09:00", now, &Utc)
                .unwrap_err()
                .contains("future")
        );
        assert!(parse_run_at_in("tomorrow", now, &Utc).is_err());
    }

    #[test]
    fn one_shot_status_labels_follow_the_lifecycle() {
        let once = Trigger::Once { run_at: 2_000 };
        let label = |enabled, runs, status| {
            schedule_status_label(enabled, Some(2_000), Some(&once), runs, status, 1_000)
        };
        assert!(label(true, 0, None).starts_with("Next "));
        assert_eq!(label(false, 0, None), "Paused");
        assert_eq!(label(false, 1, Some(RunStatus::Succeeded)), "Completed");
        assert_eq!(label(false, 1, Some(RunStatus::Failed)), "Failed");
        assert_eq!(
            schedule_status_label(false, None, Some(&once), 0, None, 3_000),
            "Expired — edit to reschedule"
        );
        assert_eq!(
            format_run_time_in(
                Utc.with_ymd_and_hms(2026, 10, 9, 15, 5, 0)
                    .unwrap()
                    .timestamp_millis(),
                &Utc
            ),
            "Oct 9, 3:05 PM"
        );
    }

    #[test]
    fn environment_labels_cover_every_shape() {
        let label = |value: JsonValue| environment_label(&value, None);
        assert_eq!(
            label(serde_json::json!({"type": "project-default"})),
            "Project default"
        );
        assert_eq!(
            label(serde_json::json!({"type": "reuse", "environmentId": "env_1"})),
            "Reuse environment env_1"
        );
        assert_eq!(
            label(
                serde_json::json!({"type": "host", "hostId": "h", "workspace": {"type": "managed-worktree", "baseBranch": {"kind": "default"}}})
            ),
            "New worktree"
        );
        assert_eq!(
            label(
                serde_json::json!({"type": "host", "hostId": "h", "workspace": {"type": "unmanaged", "path": "/srv/app"}})
            ),
            "/srv/app"
        );
        assert_eq!(
            environment_label(
                &serde_json::json!({"type": "project-default"}),
                Some("thr_1")
            ),
            "Existing thread"
        );
    }
}
