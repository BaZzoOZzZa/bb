use std::time::Instant;

use crate::app::{AppState, Effect, Focus};
use crate::composer::{Change, Composer, ComposerKey, EditMode};
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
    error: bool,
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
    let outcome = if op == EditOp::Repeat {
        repeat(&mut ctx, count)
    } else {
        let change = record(ctx.composer, op, count, register);
        let outcome = run(&mut ctx, op, count, register);
        if let Some(change) = change {
            if matches!(ctx.composer.mode, EditMode::Insert | EditMode::Replace) {
                ctx.composer.capture_insert_base();
                ctx.composer.pending_change = Some(change);
            } else {
                ctx.composer.last_change = Some(change);
            }
        }
        outcome
    };
    let one_shot = ctx.composer.one_shot
        && ctx.composer.mode == EditMode::Normal
        && !matches!(op, EditOp::Visual { .. } | EditOp::Reselect);
    if one_shot {
        ctx.composer.enter_insert();
        ctx.composer.one_shot = false;
    }
    finish(state, outcome)
}

fn finish(state: &mut AppState, outcome: Outcome) -> Vec<Effect> {
    let mut effects = Vec::new();
    if let Some(text) = outcome.copy {
        effects.push(Effect::Copy(text));
    }
    if let Some(message) = outcome.message {
        if outcome.error {
            state.keys.error(message);
        } else {
            state.keys.message(message);
        }
    }
    if outcome.to_rows
        && let Some(form) = state.new_thread.as_mut()
    {
        form.prompt_active = false;
    }
    effects
}

fn cluster_count(chars: &[char], start: usize, end: usize) -> usize {
    let mut count = 0;
    let mut pos = start;
    while pos < end {
        let next = vi::grapheme_next(chars, pos, 1);
        pos = if next > pos { next } else { pos + 1 };
        count += 1;
    }
    count
}

fn record(
    composer: &Composer,
    op: EditOp,
    count: Option<usize>,
    register: Option<char>,
) -> Option<Change> {
    if !op.repeatable() {
        return None;
    }
    let change = |op: EditOp, count: Option<usize>| {
        Some(Change {
            op,
            count,
            register,
            inserted: None,
        })
    };
    let visual = matches!(
        op,
        EditOp::VisualApply(_) | EditOp::VisualReplace(_) | EditOp::VisualJoin
    );
    if !visual {
        return change(op, count);
    }
    let range = composer.visual_range()?;
    let chars = composer.chars();
    let first = line_index(&chars, range.start);
    let last = line_index(&chars, range.end.saturating_sub(1).max(range.start));
    let lines = last - first + 1;
    match op {
        EditOp::VisualJoin => change(EditOp::Join { spaces: true }, Some(lines)),
        EditOp::VisualApply(operator) if range.linewise => {
            change(EditOp::Apply(operator, Target::Line), Some(lines))
        }
        EditOp::VisualApply(operator) if lines == 1 => change(
            EditOp::Apply(operator, Target::Motion(Motion::Right)),
            Some(cluster_count(&chars, range.start, range.end)),
        ),
        EditOp::VisualReplace(ch) if !range.linewise && lines == 1 => change(
            EditOp::ReplaceChar(ch),
            Some(cluster_count(&chars, range.start, range.end)),
        ),
        _ => None,
    }
}

fn repeat(ctx: &mut Ctx, count: Option<usize>) -> Outcome {
    let Some(change) = ctx.composer.last_change.clone() else {
        return Outcome {
            message: Some("No previous change to repeat".into()),
            ..Outcome::default()
        };
    };
    let count = count.or(change.count);
    let enters_insert = matches!(change.op, EditOp::Insert(_) | EditOp::ReplaceMode);
    let outcome = run(
        ctx,
        change.op,
        if enters_insert { None } else { count },
        change.register,
    );
    if let Some(text) = &change.inserted {
        let times = if enters_insert {
            count.unwrap_or(1).max(1)
        } else {
            1
        };
        match ctx.composer.mode {
            EditMode::Insert => {
                if !text.is_empty() {
                    ctx.composer.insert_text(&text.repeat(times), ctx.now);
                }
            }
            EditMode::Replace => {
                for ch in text.repeat(times).chars() {
                    let key = if ch == '\n' {
                        crossterm::event::KeyEvent::from(crossterm::event::KeyCode::Enter)
                    } else {
                        crossterm::event::KeyEvent::from(crossterm::event::KeyCode::Char(ch))
                    };
                    if ctx.composer.replace_key(key, ctx.now) == ComposerKey::Unhandled {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    if matches!(ctx.composer.mode, EditMode::Insert | EditMode::Replace) {
        ctx.composer.pending_change = None;
        ctx.composer.enter_normal();
    }
    ctx.composer.last_change = Some(Change { count, ..change });
    outcome
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
            if ch.is_ascii_alphanumeric() || "\"-_+*.:/".contains(ch) {
                state.keys.register = Some(ch);
                state.keys.count = count;
            } else {
                state.keys.reset();
            }
            Vec::new()
        }
        CharAct::InsertRegister => {
            let stored = state
                .registers
                .get(Some(ch))
                .map(|value| value.text.clone());
            if let Some(ctx) = context(state) {
                let text = if ch == '.' {
                    ctx.composer.last_inserted.clone()
                } else {
                    stored
                };
                if let Some(text) = text {
                    ctx.composer.insert_text(&text, ctx.now);
                }
            }
            Vec::new()
        }
        CharAct::SetMark => {
            state.keys.register = None;
            let Some(ctx) = context(state) else {
                return Vec::new();
            };
            let cursor = ctx.composer.cursor();
            if !ctx.composer.set_mark(ch, cursor) {
                state
                    .keys
                    .error("E191: Argument must be a letter or forward/backward quote");
            }
            Vec::new()
        }
        CharAct::Mark { line } => {
            state.keys.register = None;
            let Some(ctx) = context(state) else {
                return Vec::new();
            };
            let outcome = jump_to_mark(ctx.composer, ch, line);
            finish(state, outcome)
        }
        CharAct::ApplyMark { op, line } => {
            let register = state.keys.register.take();
            let Some(mut ctx) = context(state) else {
                return Vec::new();
            };
            let mut outcome = Outcome::default();
            match ctx.composer.mark(ch) {
                Some(mark) => {
                    let chars = ctx.composer.chars();
                    let pos = ctx.composer.cursor().min(chars.len());
                    let range = if line {
                        vi::lines_between(&chars, pos, mark)
                    } else {
                        Range {
                            start: pos.min(mark),
                            end: pos.max(mark),
                            linewise: false,
                        }
                    };
                    if range.linewise || range.end > range.start {
                        let range = if range.linewise {
                            range
                        } else {
                            ctx.composer.expand_chips(range)
                        };
                        apply_operator(&mut ctx, op, range, register, &mut outcome);
                    }
                }
                None => {
                    outcome.message = Some("E20: Mark not set".into());
                    outcome.error = true;
                }
            }
            finish(state, outcome)
        }
    }
}

fn jump_to_mark(composer: &mut Composer, name: char, line: bool) -> Outcome {
    let Some(mark) = composer.mark(name) else {
        return Outcome {
            message: Some("E20: Mark not set".into()),
            error: true,
            ..Outcome::default()
        };
    };
    let chars = composer.chars();
    let pos = composer.cursor();
    let target = if line {
        first_non_blank(&chars, mark)
    } else {
        mark
    };
    composer.set_mark('\'', pos);
    composer.move_to(target, target < pos);
    Outcome::default()
}

fn is_jump(motion: Motion) -> bool {
    matches!(
        motion,
        Motion::FirstLine
            | Motion::LastLine
            | Motion::MatchPair
            | Motion::ParagraphForward
            | Motion::ParagraphBackward
            | Motion::SentenceForward
            | Motion::SentenceBackward
    )
}

fn read_only(register: Option<char>) -> bool {
    matches!(register, Some('.' | ':' | '/'))
}

fn register_value(ctx: &Ctx, register: Option<char>) -> Option<RegisterValue> {
    if register == Some('.') {
        return ctx
            .composer
            .last_inserted
            .clone()
            .map(|text| RegisterValue {
                text,
                linewise: false,
            });
    }
    ctx.registers.get(register).cloned()
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
    let scan = ctx.composer.motion_chars();
    let pos = ctx.composer.cursor().min(chars.len());
    let times = count.unwrap_or(1).max(1);
    match op {
        EditOp::Insert(at) => {
            let target = match at {
                InsertAt::Before => pos,
                InsertAt::After => {
                    if pos < chars.len() && chars[pos] != '\n' {
                        vi::grapheme_next(&chars, pos, 1).max(pos + 1)
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
            if let Some(dest) = vi::motion(&scan, pos, motion, count, ctx.last_find) {
                if is_jump(motion) && dest != pos {
                    ctx.composer.set_mark('\'', pos);
                }
                ctx.composer.move_to(dest, dest < pos);
            }
        }
        EditOp::Apply(operator, target) => {
            let change = operator == Operator::Change;
            if let Some(range) = vi::target_range(&scan, pos, target, count, change, ctx.last_find)
            {
                let range = if range.linewise {
                    range
                } else {
                    ctx.composer.expand_chips(range)
                };
                apply_operator(ctx, operator, range, register, &mut outcome);
            }
        }
        EditOp::DeleteChar | EditOp::DeleteCharBefore | EditOp::Substitute => {
            let start_line = line_start(&chars, pos);
            let end_line = line_end(&chars, pos);
            let range = if op == EditOp::DeleteCharBefore {
                Range {
                    start: vi::grapheme_prev(&chars, pos, times).max(start_line),
                    end: pos,
                    linewise: false,
                }
            } else {
                Range {
                    start: pos,
                    end: vi::grapheme_next(&chars, pos, times).min(end_line),
                    linewise: false,
                }
            };
            let range = ctx.composer.expand_chips(range);
            if op == EditOp::Substitute {
                apply_operator(ctx, Operator::Change, range, register, &mut outcome);
            } else if range.end > range.start {
                apply_operator(ctx, Operator::Delete, range, register, &mut outcome);
            }
        }
        EditOp::ToggleCaseChar => {
            let end = vi::grapheme_next(&chars, pos, times).min(line_end(&chars, pos));
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
                let chars = ctx.composer.chars();
                let next = if end >= line_end(&chars, pos) {
                    vi::grapheme_prev(&chars, end, 1)
                } else {
                    end
                };
                ctx.composer.move_to(next, false);
            }
        }
        EditOp::Join { spaces } => join(ctx, pos, times.max(2) - 1, spaces),
        EditOp::Put { before } => match register_value(ctx, register) {
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
        EditOp::Reselect => {
            if !ctx.composer.reselect() {
                outcome.message = Some("No previous visual selection".into());
            }
        }
        EditOp::ReplaceMode => ctx.composer.enter_replace(),
        EditOp::UndoLine => {
            if !ctx.composer.undo_line(now) {
                outcome.message = Some("Nothing to undo on the last changed line".into());
            }
        }
        EditOp::Repeat => {}
        EditOp::VisualSwap => {
            if let EditMode::Visual { line, anchor, head } = ctx.composer.mode {
                ctx.composer.set_visual(line, head, anchor);
            }
        }
        EditOp::VisualObject(target) => {
            if let EditMode::Visual { line, .. } = ctx.composer.mode
                && let Some(range) =
                    vi::target_range(&scan, pos, target, count, false, ctx.last_find)
                && range.end > range.start
            {
                let range = ctx.composer.expand_chips(range);
                let last = vi::grapheme_start(&chars, range.end - 1);
                ctx.composer.set_visual(line, range.start, last);
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
            if let (Some(range), Some(value)) =
                (ctx.composer.visual_range(), register_value(ctx, register))
            {
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
            let line_stop = line_end(&chars, pos);
            if cluster_count(&chars, pos, line_stop) >= times {
                let end = ctx
                    .composer
                    .expand_chips(Range {
                        start: pos,
                        end: vi::grapheme_next(&chars, pos, times),
                        linewise: false,
                    })
                    .end;
                ctx.composer.checkpoint();
                let text: String = std::iter::repeat_n(ch, times).collect();
                let cursor = pos + times - 1;
                ctx.composer.replace_range(pos, end, &text, cursor, now);
                ctx.composer.move_to(cursor, false);
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
    if read_only(register)
        && matches!(
            operator,
            Operator::Yank | Operator::Delete | Operator::Change
        )
    {
        outcome.message = Some(format!(
            "E354: Invalid register name: '{}'",
            register.unwrap_or('"')
        ));
        outcome.error = true;
        return;
    }
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
