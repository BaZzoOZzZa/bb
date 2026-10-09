use bb_wire::timeline::{ItemPresentation, TimelineWorkRow};

use crate::activity_intents::{
    ExplorationRow, IntentType, intent_type, is_skill_read_intent, primary_timeline_activity_intent,
};
use crate::domain::{
    is_background_agent_task_type, is_background_command_task_type, is_namespaced_glyph,
};
use crate::title::ActivityIntentTitle;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkRowGlyph {
    CircleQuestion,
    EditFile,
    File,
    FileText,
    Folder,
    Globe,
    ListTodo,
    Lock,
    Palette,
    Puzzle,
    Search,
    Terminal,
    UserRoundPlus,
    Zap,
}

impl WorkRowGlyph {
    pub fn name(self) -> &'static str {
        match self {
            Self::CircleQuestion => "CircleQuestion",
            Self::EditFile => "EditFile",
            Self::File => "File",
            Self::FileText => "FileText",
            Self::Folder => "Folder",
            Self::Globe => "Globe",
            Self::ListTodo => "ListTodo",
            Self::Lock => "Lock",
            Self::Palette => "Palette",
            Self::Puzzle => "Puzzle",
            Self::Search => "Search",
            Self::Terminal => "Terminal",
            Self::UserRoundPlus => "UserRoundPlus",
            Self::Zap => "Zap",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedGlyph {
    Builtin(WorkRowGlyph),
    Host(String),
}

fn exploration_intent_glyph(kind: IntentType) -> WorkRowGlyph {
    match kind {
        IntentType::Search => WorkRowGlyph::Search,
        IntentType::Read => WorkRowGlyph::FileText,
        IntentType::ListFiles => WorkRowGlyph::Folder,
    }
}

pub fn activity_intent_title_glyph(entry: &ActivityIntentTitle) -> WorkRowGlyph {
    if is_skill_read_intent(&entry.intent) {
        WorkRowGlyph::Zap
    } else {
        exploration_intent_glyph(entry.intent_type)
    }
}

pub fn work_row_presentation(row: &TimelineWorkRow) -> Option<&ItemPresentation> {
    row.presentation()
}

fn fallback_glyph(row: &TimelineWorkRow) -> WorkRowGlyph {
    if let Some(exploration) = ExplorationRow::from_work(row)
        && let Some(kind) = primary_timeline_activity_intent(exploration)
            .as_ref()
            .and_then(intent_type)
    {
        return exploration_intent_glyph(kind);
    }
    match row {
        TimelineWorkRow::FileChange(_) => WorkRowGlyph::EditFile,
        TimelineWorkRow::Command(_) | TimelineWorkRow::Tool(_) => WorkRowGlyph::Terminal,
        TimelineWorkRow::FileRead(_) => WorkRowGlyph::FileText,
        TimelineWorkRow::Search(_) | TimelineWorkRow::WebSearch(_) => WorkRowGlyph::Search,
        TimelineWorkRow::PlanSteps(_) => WorkRowGlyph::ListTodo,
        TimelineWorkRow::Extension(_) | TimelineWorkRow::Form(_) => WorkRowGlyph::Puzzle,
        TimelineWorkRow::WebFetch(_) => WorkRowGlyph::Globe,
        TimelineWorkRow::ImageGeneration(_) => WorkRowGlyph::Palette,
        TimelineWorkRow::ImageView(_) => WorkRowGlyph::File,
        TimelineWorkRow::Delegation(_) => WorkRowGlyph::UserRoundPlus,
        TimelineWorkRow::Workflow(row) => {
            if is_background_command_task_type(&row.task_type) {
                WorkRowGlyph::Terminal
            } else if is_background_agent_task_type(&row.task_type) {
                WorkRowGlyph::UserRoundPlus
            } else {
                WorkRowGlyph::ListTodo
            }
        }
        TimelineWorkRow::Approval(_) => WorkRowGlyph::Lock,
        TimelineWorkRow::Question(_) => WorkRowGlyph::CircleQuestion,
        TimelineWorkRow::Unknown(_) => WorkRowGlyph::Puzzle,
    }
}

fn is_skill_read_command_row(row: &TimelineWorkRow) -> bool {
    matches!(row, TimelineWorkRow::Command(command) if command.activity_intents.iter().any(is_skill_read_intent))
}

pub fn work_row_glyph(
    row: &TimelineWorkRow,
    is_host_glyph: impl Fn(&str) -> bool,
) -> ResolvedGlyph {
    if is_skill_read_command_row(row) {
        return ResolvedGlyph::Builtin(WorkRowGlyph::Zap);
    }
    if let Some(glyph) =
        work_row_presentation(row).map(|presentation| presentation.icon.glyph.as_str())
        && is_host_glyph(glyph)
    {
        return ResolvedGlyph::Host(glyph.to_owned());
    }
    ResolvedGlyph::Builtin(fallback_glyph(row))
}

pub fn work_row_plugin_glyph(row: &TimelineWorkRow) -> Option<&str> {
    if is_skill_read_command_row(row) {
        return None;
    }
    work_row_presentation(row)
        .map(|presentation| presentation.icon.glyph.as_str())
        .filter(|glyph| is_namespaced_glyph(glyph))
}

#[cfg(test)]
mod tests {
    use bb_wire::timeline::{
        ActivityIntent, CommandWorkRow, ItemPresentation, PresentationIcon, WorkflowWorkRow,
    };

    use super::*;

    fn command(intents: Vec<ActivityIntent>) -> TimelineWorkRow {
        TimelineWorkRow::Command(Box::new(CommandWorkRow {
            activity_intents: intents,
            ..CommandWorkRow::default()
        }))
    }

    #[test]
    fn picks_exploration_glyphs_from_primary_intent() {
        let read = command(vec![
            ActivityIntent::Generic {
                command: "cd x".into(),
            },
            ActivityIntent::Read {
                command: "cat a".into(),
                name: "a".into(),
                path: Some("a".into()),
            },
        ]);
        assert_eq!(
            work_row_glyph(&read, |_| false),
            ResolvedGlyph::Builtin(WorkRowGlyph::FileText)
        );
        assert_eq!(
            work_row_glyph(&command(Vec::new()), |_| false),
            ResolvedGlyph::Builtin(WorkRowGlyph::Terminal)
        );
    }

    #[test]
    fn prefers_skill_reads_then_host_glyphs() {
        let mut row = CommandWorkRow {
            activity_intents: vec![ActivityIntent::Read {
                command: "cat".into(),
                name: "SKILL.md".into(),
                path: Some("/x/skills/demo/SKILL.md".into()),
            }],
            presentation: Some(ItemPresentation {
                icon: PresentationIcon {
                    glyph: "acme/rocket".into(),
                },
                ..ItemPresentation::default()
            }),
            ..CommandWorkRow::default()
        };
        let skill = TimelineWorkRow::Command(Box::new(row.clone()));
        assert_eq!(
            work_row_glyph(&skill, |_| true),
            ResolvedGlyph::Builtin(WorkRowGlyph::Zap)
        );
        assert_eq!(work_row_plugin_glyph(&skill), None);
        row.activity_intents.clear();
        let plain = TimelineWorkRow::Command(Box::new(row));
        assert_eq!(
            work_row_glyph(&plain, |glyph| glyph == "acme/rocket"),
            ResolvedGlyph::Host("acme/rocket".into())
        );
        assert_eq!(work_row_plugin_glyph(&plain), Some("acme/rocket"));
    }

    #[test]
    fn maps_background_workflow_task_types() {
        let workflow = |task_type: &str| {
            TimelineWorkRow::Workflow(Box::new(WorkflowWorkRow {
                task_type: task_type.into(),
                ..WorkflowWorkRow::default()
            }))
        };
        assert_eq!(
            work_row_glyph(&workflow("local_bash"), |_| false),
            ResolvedGlyph::Builtin(WorkRowGlyph::Terminal)
        );
        assert_eq!(
            work_row_glyph(&workflow("local_subagent"), |_| false),
            ResolvedGlyph::Builtin(WorkRowGlyph::UserRoundPlus)
        );
        assert_eq!(
            work_row_glyph(&workflow("local_workflow"), |_| false),
            ResolvedGlyph::Builtin(WorkRowGlyph::ListTodo)
        );
    }
}
