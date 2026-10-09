use std::time::Instant;

use crate::app::{AppState, Effect, Focus};
use crate::composer::{Composer, EditMode};
use crate::evil::{RegisterValue, Registers};
use crate::vi::{
    self, CharAct, EditOp, InsertAt, LastFind, Motion, Operator, Range, Target, line_end,
    line_index, line_start,
};

struct Ctx<'a> {
    composer: &'a mut Composer,
    registers: &'a mut Registers,
    last_find: &'a mut LastFind,
    now: Instant,
    new_thread: bool,
}

#[derive(Default)]
struct Outcome {
    copy: Option<String>,
    message: Option<String>,
    to_rows: bool,
}

fn context(state: &mut AppState) -> Option<Ctx<'_>> {
    let now = state.clock;
    let (composer, new_thread) = match state.focus {
        Focus::NewThread => (
            state
                .new_thread
                .as_mut()
                .filter(|form| form.prompt_active)
                .map(|form| &mut form.composer)?,
            true,
        ),
        Focus::Composer => {
            let thread_id = state.open_thread.as_deref()?;
            (state.composers.get_mut(thread_id)?, false)
        }
        _ => return None,
    };
    Some(Ctx {
        composer,
        registers: &mut state.registers,
        last_find: &mut state.keys.last_find,
        now,
        new_thread,
    })
}

pub(crate) fn apply_edit(state: &mut AppState, op: EditOp, count: Option<usize>) -> Vec<Effect> {
    let register = state.keys.register.take();
    let Some(mut ctx) = context(state) else {
        return Vec::new();
    };
    let outcome = run(&mut ctx, op, count, register);
    let one_shot = ctx.composer.one_shot
        && ctx.composer.mode == EditMode::Normal
        && !matches!(op, EditOp::Visual { .. });
    if one_shot {
        ctx.composer.enter_insert();
        ctx.composer.one_shot = false;
    }
    let mut effects = Vec::new();
    if let Some(text) = outcome.copy {
        effects.push(Effect::Copy(text));
    }
    if let Some(message) = outcome.message {
        state.keys.message(message);
    }
    if outcome.to_rows
        && let Some(form) = state.new_thread.as_mut()
    {
        form.prompt_active = false;
    }
    effects
}

pub(crate) fn char_act(
    state: &mut AppState,
    act: CharAct,
    ch: char,
    count: Option<usize>,
) -> Vec<Effect> {
    match act {
        CharAct::Find { forward, till } => apply_edit(
            state,
            EditOp::Move(Motion::Find { forward, till, ch }),
            count,
        ),
        CharAct::ApplyFind { op, forward, till } => apply_edit(
            state,
            EditOp::Apply(op, Target::Motion(Motion::Find { forward, till, ch })),
            count,
        ),
        CharAct::Replace => apply_edit(state, EditOp::ReplaceChar(ch), count),
        CharAct::VisualReplace => apply_edit(state, EditOp::VisualReplace(ch), count),
        CharAct::Register => {
            if ch.is_ascii_alphanumeric() || "\"-_+*".contains(ch) {
                state.keys.register = Some(ch);
                state.keys.count = count;
            } else {
                state.keys.reset();
            }
            Vec::new()
        }
        CharAct::InsertRegister => {
            let text = state
                .registers
                .get(Some(ch))
                .map(|value| value.text.clone());
            if let (Some(text), Some(ctx)) = (text, context(state)) {
                ctx.composer.insert_text(&text, ctx.now);
            }
            Vec::new()
        }
    }
}

fn value(chars: &[char], range: Range) -> RegisterValue {
    let mut text: String = chars[range.start..range.end].iter().collect();
    if range.linewise {
        if let Some(stripped) = text.strip_prefix('\n')
            && range.end == chars.len()
        {
            text = stripped.to_owned();
        }
        if !text.ends_with('\n') {
            text.push('\n');
        }
    }
    RegisterValue {
        text,
        linewise: range.linewise,
    }
}

fn first_non_blank(chars: &[char], pos: usize) -> usize {
    let start = line_start(chars, pos);
    let end = line_end(chars, pos);
    (start..end)
        .find(|index| !chars[*index].is_whitespace())
        .unwrap_or(start)
}

fn run(ctx: &mut Ctx, op: EditOp, count: Option<usize>, register: Option<char>) -> Outcome {
    let mut outcome = Outcome::default();
    let now = ctx.now;
    let chars = ctx.composer.chars();
    let pos = ctx.composer.cursor().min(chars.len());
    let times = count.unwrap_or(1).max(1);
    match op {
        EditOp::Insert(at) => {
            let target = match at {
                InsertAt::Before => pos,
                InsertAt::After => {
                    if pos < chars.len() && chars[pos] != '\n' {
                        pos + 1
                    } else {
                        pos
                    }
                }
                InsertAt::LineStart => first_non_blank(&chars, pos),
                InsertAt::LineEnd => line_end(&chars, pos),
                InsertAt::Below | InsertAt::Above => pos,
                InsertAt::Last => ctx.composer.last_insert.unwrap_or(chars.len()),
            };
            ctx.composer.enter_insert();
            match at {
                InsertAt::Below => {
                    let end = line_end(&chars, pos);
                    ctx.composer.replace_range(end, end, "\n", end + 1, now);
                }
                InsertAt::Above => {
                    let start = line_start(&chars, pos);
                    ctx.composer.replace_range(start, start, "\n", start, now);
                }
                _ => ctx.composer.move_to(target, false),
            }
        }
        EditOp::Move(motion) => {
            if ctx.new_thread
                && motion == Motion::Up
                && line_index(&chars, pos) == 0
                && !matches!(ctx.composer.mode, EditMode::Visual { .. })
            {
                outcome.to_rows = true;
                return outcome;
            }
            if let Some(dest) = vi::motion(&chars, pos, motion, count, ctx.last_find) {
                ctx.composer.move_to(dest, dest < pos);
            }
        }
        EditOp::Apply(operator, target) => {
            let change = operator == Operator::Change;
            if let Some(range) = vi::target_range(&chars, pos, target, count, change, ctx.last_find)
            {
                apply_operator(ctx, operator, range, register, &mut outcome);
            }
        }
        EditOp::DeleteChar | EditOp::DeleteCharBefore | EditOp::Substitute => {
            let start_line = line_start(&chars, pos);
            let end_line = line_end(&chars, pos);
            let range = if op == EditOp::DeleteCharBefore {
                Range {
                    start: pos.saturating_sub(times).max(start_line),
                    end: pos,
                    linewise: false,
                }
            } else {
                Range {
                    start: pos,
                    end: (pos + times).min(end_line),
                    linewise: false,
                }
            };
            if op == EditOp::Substitute {
                apply_operator(ctx, Operator::Change, range, register, &mut outcome);
            } else if range.end > range.start {
                apply_operator(ctx, Operator::Delete, range, register, &mut outcome);
            }
        }
        EditOp::ToggleCaseChar => {
            let end = (pos + times).min(line_end(&chars, pos));
            if end > pos {
                let text: String = chars[pos..end].iter().collect();
                ctx.composer.checkpoint();
                ctx.composer.replace_range(
                    pos,
                    end,
                    &vi::transform(&text, Operator::ToggleCase),
                    end,
                    now,
                );
                ctx.composer.move_to(end, false);
            }
        }
        EditOp::Join { spaces } => join(ctx, pos, times.max(2) - 1, spaces),
        EditOp::Put { before } => match ctx.registers.get(register).cloned() {
            Some(value) => put(ctx, &chars, pos, &value, before, times),
            None => {
                outcome.message = Some(format!(
                    "E353: Nothing in register {}",
                    register.unwrap_or('"')
                ))
            }
        },
        EditOp::Undo => {
            let mut done = 0;
            for _ in 0..times {
                if ctx.composer.undo(now) {
                    done += 1;
                }
            }
            if done == 0 {
                outcome.message = Some("Already at oldest change".into());
            }
        }
        EditOp::Redo => {
            let mut done = 0;
            for _ in 0..times {
                if ctx.composer.redo(now) {
                    done += 1;
                }
            }
            if done == 0 {
                outcome.message = Some("Already at newest change".into());
            }
        }
        EditOp::Visual { line } => match ctx.composer.mode {
            EditMode::Visual {
                line: current,
                anchor,
                head,
            } => {
                if current == line {
                    ctx.composer.enter_normal();
                } else {
                    ctx.composer.set_visual(line, anchor, head);
                }
            }
            _ => ctx.composer.start_visual(line),
        },
        EditOp::VisualSwap => {
            if let EditMode::Visual { line, anchor, head } = ctx.composer.mode {
                ctx.composer.set_visual(line, head, anchor);
            }
        }
        EditOp::VisualObject(target) => {
            if let EditMode::Visual { line, .. } = ctx.composer.mode
                && let Some(range) =
                    vi::target_range(&chars, pos, target, count, false, ctx.last_find)
                && range.end > range.start
            {
                ctx.composer.set_visual(line, range.start, range.end - 1);
            }
        }
        EditOp::VisualApply(operator) => {
            if let Some(range) = ctx.composer.visual_range() {
                ctx.composer.enter_normal();
                apply_operator(ctx, operator, range, register, &mut outcome);
            }
        }
        EditOp::VisualJoin => {
            if let EditMode::Visual { anchor, head, .. } = ctx.composer.mode {
                let lines =
                    line_index(&chars, anchor.max(head)) - line_index(&chars, anchor.min(head));
                ctx.composer.enter_normal();
                join(ctx, anchor.min(head), lines.max(1), true);
            }
        }
        EditOp::VisualPut => {
            if let (Some(range), Some(value)) = (
                ctx.composer.visual_range(),
                ctx.registers.get(register).cloned(),
            ) {
                ctx.composer.enter_normal();
                ctx.composer.checkpoint();
                let mut text = value.text.clone();
                if value.linewise && !range.linewise {
                    text = text.trim_end_matches('\n').to_owned();
                }
                let cursor = range.start + text.chars().count().saturating_sub(1);
                ctx.composer
                    .replace_range(range.start, range.end, &text, cursor, now);
                ctx.composer.move_to(cursor, false);
            }
        }
        EditOp::ReplaceChar(ch) => {
            let end = pos + times;
            if end <= line_end(&chars, pos) {
                ctx.composer.checkpoint();
                let text: String = std::iter::repeat_n(ch, times).collect();
                ctx.composer.replace_range(pos, end, &text, end - 1, now);
                ctx.composer.move_to(end - 1, false);
            }
        }
        EditOp::VisualReplace(ch) => {
            if let Some(range) = ctx.composer.visual_range() {
                ctx.composer.enter_normal();
                ctx.composer.checkpoint();
                let text: String = chars[range.start..range.end]
                    .iter()
                    .map(|current| if *current == '\n' { '\n' } else { ch })
                    .collect();
                ctx.composer
                    .replace_range(range.start, range.end, &text, range.start, now);
                ctx.composer.move_to(range.start, false);
            }
        }
        EditOp::HistoryPrevious | EditOp::HistoryNext => {
            for _ in 0..times {
                ctx.composer.history_step(op == EditOp::HistoryPrevious);
            }
            let chars = ctx.composer.chars();
            ctx.composer.move_to(chars.len(), false);
        }
    }
    outcome
}

fn apply_operator(
    ctx: &mut Ctx,
    operator: Operator,
    range: Range,
    register: Option<char>,
    outcome: &mut Outcome,
) {
    let now = ctx.now;
    let chars = ctx.composer.chars();
    let range = Range {
        start: range.start.min(chars.len()),
        end: range.end.min(chars.len()),
        linewise: range.linewise,
    };
    match operator {
        Operator::Yank => {
            outcome.copy = ctx.registers.yank(register, value(&chars, range));
            if !range.linewise {
                ctx.composer.move_to(range.start, true);
            } else if let EditMode::Visual { .. } = ctx.composer.mode {
                ctx.composer.enter_normal();
            }
        }
        Operator::Delete => {
            ctx.composer.checkpoint();
            outcome.copy = ctx.registers.delete(register, value(&chars, range));
            ctx.composer
                .replace_range(range.start, range.end, "", range.start, now);
            let chars = ctx.composer.chars();
            let cursor = if range.linewise {
                first_non_blank(&chars, range.start.min(chars.len()))
            } else {
                range.start
            };
            ctx.composer.move_to(cursor, true);
        }
        Operator::Change => {
            ctx.composer.enter_insert();
            outcome.copy = ctx.registers.delete(register, value(&chars, range));
            let (start, end) = if range.linewise {
                let start =
                    if range.end == chars.len() && range.start > 0 && chars[range.start] == '\n' {
                        range.start + 1
                    } else {
                        range.start
                    };
                let end = if range.end > start
                    && chars.get(range.end - 1) == Some(&'\n')
                    && range.end != chars.len()
                {
                    range.end - 1
                } else {
                    range.end
                };
                (start, end)
            } else {
                (range.start, range.end)
            };
            ctx.composer.replace_range(start, end, "", start, now);
            ctx.composer.move_to(start, false);
        }
        Operator::Indent | Operator::Outdent => {
            let start = line_start(&chars, range.start);
            let end = line_end(&chars, range.end.saturating_sub(1).max(range.start));
            let text: String = chars[start..end].iter().collect();
            let next = vi::transform(&text, operator);
            if next != text {
                ctx.composer.checkpoint();
                ctx.composer.replace_range(start, end, &next, start, now);
            }
            let chars = ctx.composer.chars();
            ctx.composer.move_to(first_non_blank(&chars, start), false);
        }
        Operator::Lower | Operator::Upper | Operator::ToggleCase => {
            let text: String = chars[range.start..range.end].iter().collect();
            let next = vi::transform(&text, operator);
            if next != text {
                ctx.composer.checkpoint();
                ctx.composer
                    .replace_range(range.start, range.end, &next, range.start, now);
            }
            ctx.composer.move_to(range.start, true);
        }
    }
}

fn join(ctx: &mut Ctx, pos: usize, joins: usize, spaces: bool) {
    let now = ctx.now;
    let mut changed = false;
    let mut cursor = pos;
    for _ in 0..joins {
        let chars = ctx.composer.chars();
        let end = line_end(&chars, cursor);
        if end >= chars.len() {
            break;
        }
        let mut next = end + 1;
        while spaces && next < chars.len() && chars[next] != '\n' && chars[next].is_whitespace() {
            next += 1;
        }
        let separator = if spaces
            && next < chars.len()
            && chars[next] != '\n'
            && end > line_start(&chars, end)
            && !chars[end - 1].is_whitespace()
        {
            " "
        } else {
            ""
        };
        if !changed {
            ctx.composer.checkpoint();
            changed = true;
        }
        ctx.composer.replace_range(end, next, separator, end, now);
        cursor = end;
    }
    if changed {
        ctx.composer.move_to(cursor, false);
    }
}

fn put(
    ctx: &mut Ctx,
    chars: &[char],
    pos: usize,
    value: &RegisterValue,
    before: bool,
    times: usize,
) {
    let now = ctx.now;
    ctx.composer.checkpoint();
    let text = value.text.repeat(times);
    if value.linewise {
        let body = if text.ends_with('\n') {
            text.clone()
        } else {
            format!("{text}\n")
        };
        if before {
            let at = line_start(chars, pos);
            ctx.composer.replace_range(at, at, &body, at, now);
            let chars = ctx.composer.chars();
            ctx.composer.move_to(first_non_blank(&chars, at), false);
        } else {
            let end = line_end(chars, pos);
            if end >= chars.len() {
                let inserted = format!("\n{}", body.trim_end_matches('\n'));
                ctx.composer
                    .replace_range(end, end, &inserted, end + 1, now);
                let chars = ctx.composer.chars();
                ctx.composer
                    .move_to(first_non_blank(&chars, end + 1), false);
            } else {
                ctx.composer
                    .replace_range(end + 1, end + 1, &body, end + 1, now);
                let chars = ctx.composer.chars();
                ctx.composer
                    .move_to(first_non_blank(&chars, end + 1), false);
            }
        }
        return;
    }
    let at = if before || chars.get(pos).is_none_or(|ch| *ch == '\n') {
        pos
    } else {
        pos + 1
    };
    let length = text.chars().count();
    let cursor = at + length.saturating_sub(1);
    ctx.composer.replace_range(at, at, &text, cursor, now);
    ctx.composer.move_to(cursor, false);
}
