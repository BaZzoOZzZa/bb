use std::mem::discriminant;

use bb_wire::timeline::{
    SystemOperationKind, SystemRowKind, TimelineRow, TimelineWorkRow, TurnSummaryDetailsQuery,
};

use crate::timeline_merge::row_id;

fn is_deferrable_operation(row: &bb_wire::timeline::SystemRow) -> bool {
    row.system_kind == SystemRowKind::Operation
        && row.operation_kind != Some(SystemOperationKind::ParentChange)
}

pub fn deferred_timeline_content_item_id(row: &TimelineRow) -> Option<&str> {
    match row {
        TimelineRow::Work(work) if work.content_deferred() => match work {
            TimelineWorkRow::Command(row) => Some(&row.call_id),
            TimelineWorkRow::Tool(row) => Some(&row.call_id),
            TimelineWorkRow::FileChange(row) => Some(&row.call_id),
            _ => None,
        },
        TimelineRow::System(row) if is_deferrable_operation(row) && row.content_deferred => {
            row.reasoning_id.as_deref()
        }
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TurnDetailsKey {
    pub thread_id: String,
    pub turn_id: String,
    pub source_seq_start: i64,
    pub source_seq_end: i64,
    pub item_id: Option<String>,
}

impl TurnDetailsKey {
    pub fn for_turn(thread_id: &str, row: &TimelineRow) -> Option<Self> {
        let base = row.base()?;
        Some(Self {
            thread_id: thread_id.to_owned(),
            turn_id: base.turn_id.clone()?,
            source_seq_start: base.source_seq_start,
            source_seq_end: base.source_seq_end,
            item_id: None,
        })
    }

    pub fn for_item(thread_id: &str, row: &TimelineRow, item_id: &str) -> Option<Self> {
        Self::for_turn(thread_id, row).map(|key| Self {
            item_id: Some(item_id.to_owned()),
            ..key
        })
    }

    pub fn for_deferred_row(thread_id: &str, row: &TimelineRow) -> Option<Self> {
        let item_id = deferred_timeline_content_item_id(row)?;
        Self::for_item(thread_id, row, item_id)
    }

    pub fn query(&self, before_cursor: Option<String>) -> TurnSummaryDetailsQuery {
        TurnSummaryDetailsQuery {
            before_cursor,
            defer_content: Some(true),
            item_id: self.item_id.clone(),
            turn_id: self.turn_id.clone(),
            source_seq_start: self.source_seq_start,
            source_seq_end: self.source_seq_end,
        }
    }
}

fn find_loaded_row<'a>(
    row: &TimelineRow,
    item_id: &str,
    loaded: &'a [TimelineRow],
) -> Option<&'a TimelineRow> {
    let id = row_id(row);
    if let Some(found) = loaded.iter().find(|candidate| row_id(candidate) == id) {
        return Some(found);
    }
    loaded.iter().find(|candidate| match (row, candidate) {
        (TimelineRow::System(_), TimelineRow::System(candidate)) => {
            candidate.reasoning_id.as_deref() == Some(item_id)
        }
        (TimelineRow::System(_), _) => false,
        (TimelineRow::Work(work), TimelineRow::Work(candidate)) => {
            if discriminant(work) != discriminant(candidate) {
                return false;
            }
            let call_id = match candidate {
                TimelineWorkRow::Command(row) => &row.call_id,
                TimelineWorkRow::Tool(row) => &row.call_id,
                TimelineWorkRow::FileChange(row) => &row.call_id,
                TimelineWorkRow::WebSearch(row) => &row.call_id,
                TimelineWorkRow::WebFetch(row) => &row.call_id,
                TimelineWorkRow::ImageGeneration(row) => &row.call_id,
                TimelineWorkRow::ImageView(row) => &row.call_id,
                TimelineWorkRow::FileRead(row) => &row.call_id,
                TimelineWorkRow::Search(row) => &row.call_id,
                TimelineWorkRow::PlanSteps(row) => &row.call_id,
                TimelineWorkRow::Extension(row) => &row.call_id,
                TimelineWorkRow::Delegation(row) => &row.call_id,
                _ => return false,
            };
            if call_id != item_id {
                return false;
            }
            match (work, candidate) {
                (TimelineWorkRow::FileChange(row), TimelineWorkRow::FileChange(candidate)) => {
                    row.change.path == candidate.change.path
                }
                _ => true,
            }
        }
        _ => false,
    })
}

pub fn resolve_deferred_timeline_content(
    row: &TimelineRow,
    loaded: &[TimelineRow],
) -> Option<TimelineRow> {
    let Some(item_id) = deferred_timeline_content_item_id(row) else {
        return Some(row.clone());
    };
    let found = find_loaded_row(row, item_id, loaded)?;
    match (row, found) {
        (TimelineRow::System(row), TimelineRow::System(found)) => {
            let mut resolved = (**row).clone();
            resolved.content_deferred = false;
            resolved.detail = found.detail.clone();
            Some(TimelineRow::System(Box::new(resolved)))
        }
        (TimelineRow::Work(work), TimelineRow::Work(found)) => match (work, found) {
            (TimelineWorkRow::Command(row), TimelineWorkRow::Command(found)) => {
                let mut resolved = (**row).clone();
                resolved.content_deferred = false;
                resolved.command = found.command.clone();
                resolved.output = found.output.clone();
                if found.output_preview.is_some() {
                    resolved.output_preview = found.output_preview.clone();
                }
                Some(TimelineRow::Work(TimelineWorkRow::Command(Box::new(
                    resolved,
                ))))
            }
            (TimelineWorkRow::Tool(row), TimelineWorkRow::Tool(found)) => {
                let mut resolved = (**row).clone();
                resolved.content_deferred = false;
                resolved.output = found.output.clone();
                if found.output_preview.is_some() {
                    resolved.output_preview = found.output_preview.clone();
                }
                Some(TimelineRow::Work(TimelineWorkRow::Tool(Box::new(resolved))))
            }
            (TimelineWorkRow::FileChange(row), TimelineWorkRow::FileChange(found)) => {
                let mut resolved = (**row).clone();
                resolved.content_deferred = false;
                resolved.change = found.change.clone();
                resolved.stderr = found.stderr.clone();
                resolved.stdout = found.stdout.clone();
                Some(TimelineRow::Work(TimelineWorkRow::FileChange(Box::new(
                    resolved,
                ))))
            }
            _ => None,
        },
        _ => None,
    }
}
