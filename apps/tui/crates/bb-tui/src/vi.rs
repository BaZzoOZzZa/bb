#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Motion {
    Left,
    Right,
    Up,
    Down,
    WordForward,
    WordBackward,
    WordEnd,
    WordEndBackward,
    BigWordForward,
    BigWordBackward,
    BigWordEnd,
    BigWordEndBackward,
    LineStart,
    FirstNonBlank,
    LineEnd,
    LastNonBlank,
    FirstLine,
    LastLine,
    ParagraphForward,
    ParagraphBackward,
    SentenceForward,
    SentenceBackward,
    MatchPair,
    Find { forward: bool, till: bool, ch: char },
    RepeatFind { reverse: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Operator {
    Delete,
    Change,
    Yank,
    Indent,
    Outdent,
    Lower,
    Upper,
    ToggleCase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Object {
    Word,
    BigWord,
    Sentence,
    Paragraph,
    Quote(char),
    Pair(char, char),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Target {
    Motion(Motion),
    Line,
    Inner(Object),
    Around(Object),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InsertAt {
    Before,
    After,
    LineStart,
    LineEnd,
    Below,
    Above,
    Last,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EditOp {
    Insert(InsertAt),
    Move(Motion),
    Apply(Operator, Target),
    DeleteChar,
    DeleteCharBefore,
    Substitute,
    ToggleCaseChar,
    Join { spaces: bool },
    Put { before: bool },
    Undo,
    Redo,
    Visual { line: bool },
    Reselect,
    ReplaceMode,
    UndoLine,
    Repeat,
    VisualSwap,
    VisualApply(Operator),
    VisualObject(Target),
    VisualJoin,
    VisualPut,
    ReplaceChar(char),
    VisualReplace(char),
    HistoryPrevious,
    HistoryNext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CharAct {
    Find {
        forward: bool,
        till: bool,
    },
    ApplyFind {
        op: Operator,
        forward: bool,
        till: bool,
    },
    Replace,
    VisualReplace,
    Register,
    InsertRegister,
    SetMark,
    Mark {
        line: bool,
    },
    ApplyMark {
        op: Operator,
        line: bool,
    },
}

impl CharAct {
    pub fn label(self) -> &'static str {
        match self {
            Self::Find {
                forward: true,
                till: false,
            } => "Find a character forward",
            Self::Find {
                forward: false,
                till: false,
            } => "Find a character backward",
            Self::Find {
                forward: true,
                till: true,
            } => "Till a character forward",
            Self::Find {
                forward: false,
                till: true,
            } => "Till a character backward",
            Self::ApplyFind { .. } => "Operate up to a character",
            Self::Replace | Self::VisualReplace => "Replace with a character",
            Self::Register => "Use a register",
            Self::InsertRegister => "Insert a register",
            Self::SetMark => "Set a mark",
            Self::Mark { line: true } => "Go to a mark's line",
            Self::Mark { line: false } => "Go to a mark",
            Self::ApplyMark { .. } => "Operate up to a mark",
        }
    }
}

impl Operator {
    pub fn label(self) -> &'static str {
        match self {
            Self::Delete => "delete",
            Self::Change => "change",
            Self::Yank => "yank",
            Self::Indent => "indent",
            Self::Outdent => "outdent",
            Self::Lower => "lowercase",
            Self::Upper => "uppercase",
            Self::ToggleCase => "toggle case",
        }
    }
}

impl Motion {
    pub fn label(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Up => "line up",
            Self::Down => "line down",
            Self::WordForward => "next word",
            Self::WordBackward => "previous word",
            Self::WordEnd => "end of word",
            Self::WordEndBackward => "end of previous word",
            Self::BigWordForward => "next WORD",
            Self::BigWordBackward => "previous WORD",
            Self::BigWordEnd => "end of WORD",
            Self::BigWordEndBackward => "end of previous WORD",
            Self::LineStart => "line start",
            Self::FirstNonBlank => "first non-blank",
            Self::LineEnd => "line end",
            Self::LastNonBlank => "last non-blank",
            Self::FirstLine => "first line",
            Self::LastLine => "last line",
            Self::ParagraphForward => "next paragraph",
            Self::ParagraphBackward => "previous paragraph",
            Self::SentenceForward => "next sentence",
            Self::SentenceBackward => "previous sentence",
            Self::MatchPair => "matching bracket",
            Self::Find { .. } => "find character",
            Self::RepeatFind { reverse: false } => "repeat find",
            Self::RepeatFind { reverse: true } => "repeat find backward",
        }
    }

    fn linewise(self) -> bool {
        matches!(
            self,
            Self::Up | Self::Down | Self::FirstLine | Self::LastLine
        )
    }

    fn inclusive(self) -> bool {
        matches!(
            self,
            Self::WordEnd
                | Self::WordEndBackward
                | Self::BigWordEnd
                | Self::BigWordEndBackward
                | Self::LineEnd
                | Self::LastNonBlank
                | Self::MatchPair
                | Self::Find { .. }
                | Self::RepeatFind { .. }
        )
    }
}

impl Object {
    pub fn label(self) -> &'static str {
        match self {
            Self::Word => "word",
            Self::BigWord => "WORD",
            Self::Sentence => "sentence",
            Self::Paragraph => "paragraph",
            Self::Quote(_) => "quoted text",
            Self::Pair(..) => "bracketed text",
        }
    }
}

impl EditOp {
    pub fn description(self) -> String {
        match self {
            Self::Insert(InsertAt::Before) => "Insert before the cursor".into(),
            Self::Insert(InsertAt::After) => "Insert after the cursor".into(),
            Self::Insert(InsertAt::LineStart) => "Insert at the line start".into(),
            Self::Insert(InsertAt::LineEnd) => "Insert at the line end".into(),
            Self::Insert(InsertAt::Below) => "Open a line below".into(),
            Self::Insert(InsertAt::Above) => "Open a line above".into(),
            Self::Insert(InsertAt::Last) => "Insert where insert was last left".into(),
            Self::Move(motion) => format!("Move: {}", motion.label()),
            Self::Apply(op, Target::Line) => format!("{} line", capitalize(op.label())),
            Self::Apply(op, Target::Motion(motion)) => {
                format!("{} to {}", capitalize(op.label()), motion.label())
            }
            Self::Apply(op, Target::Inner(object)) => {
                format!("{} inner {}", capitalize(op.label()), object.label())
            }
            Self::Apply(op, Target::Around(object)) => {
                format!("{} around {}", capitalize(op.label()), object.label())
            }
            Self::DeleteChar => "Delete the character".into(),
            Self::DeleteCharBefore => "Delete the character before".into(),
            Self::Substitute => "Substitute the character".into(),
            Self::ToggleCaseChar => "Toggle case".into(),
            Self::Join { spaces: true } => "Join lines".into(),
            Self::Join { spaces: false } => "Join lines without spaces".into(),
            Self::Put { before: false } => "Put after".into(),
            Self::Put { before: true } => "Put before".into(),
            Self::Undo => "Undo".into(),
            Self::Redo => "Redo".into(),
            Self::Visual { line: false } => "Visual state".into(),
            Self::Visual { line: true } => "Visual-line state".into(),
            Self::Reselect => "Reselect the last visual selection".into(),
            Self::ReplaceMode => "Replace state".into(),
            Self::UndoLine => "Undo all changes on the last changed line".into(),
            Self::Repeat => "Repeat the last change".into(),
            Self::VisualSwap => "Other end of the selection".into(),
            Self::VisualApply(op) => format!("{} the selection", capitalize(op.label())),
            Self::VisualObject(Target::Inner(object)) => format!("Select inner {}", object.label()),
            Self::VisualObject(Target::Around(object)) => {
                format!("Select around {}", object.label())
            }
            Self::VisualObject(_) => "Select".into(),
            Self::VisualJoin => "Join the selected lines".into(),
            Self::VisualPut => "Replace the selection with a register".into(),
            Self::ReplaceChar(_) | Self::VisualReplace(_) => "Replace with a character".into(),
            Self::HistoryPrevious => "Previous prompt history entry".into(),
            Self::HistoryNext => "Next prompt history entry".into(),
        }
    }

    pub fn repeatable(self) -> bool {
        !matches!(
            self,
            Self::Move(_)
                | Self::Apply(Operator::Yank, _)
                | Self::Undo
                | Self::Redo
                | Self::Visual { .. }
                | Self::Reselect
                | Self::Repeat
                | Self::VisualSwap
                | Self::VisualApply(Operator::Yank)
                | Self::VisualObject(_)
                | Self::VisualPut
                | Self::UndoLine
                | Self::HistoryPrevious
                | Self::HistoryNext
        )
    }
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    pub start: usize,
    pub end: usize,
    pub linewise: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LastFind {
    pub forward: bool,
    pub till: bool,
    pub ch: Option<char>,
}

pub const CHIP: char = '\u{F8FF}';

fn class(ch: char, big: bool) -> u8 {
    if ch.is_whitespace() {
        0
    } else if ch == CHIP {
        3
    } else if big || ch.is_alphanumeric() || ch == '_' {
        1
    } else {
        2
    }
}

pub fn line_start(chars: &[char], pos: usize) -> usize {
    let pos = pos.min(chars.len());
    chars[..pos]
        .iter()
        .rposition(|ch| *ch == '\n')
        .map_or(0, |index| index + 1)
}

pub fn line_end(chars: &[char], pos: usize) -> usize {
    let pos = pos.min(chars.len());
    chars[pos..]
        .iter()
        .position(|ch| *ch == '\n')
        .map_or(chars.len(), |index| pos + index)
}

pub fn line_index(chars: &[char], pos: usize) -> usize {
    chars[..pos.min(chars.len())]
        .iter()
        .filter(|ch| **ch == '\n')
        .count()
}

fn line_count(chars: &[char]) -> usize {
    chars.iter().filter(|ch| **ch == '\n').count() + 1
}

fn nth_line_start(chars: &[char], line: usize) -> usize {
    if line == 0 {
        return 0;
    }
    let mut seen = 0;
    for (index, ch) in chars.iter().enumerate() {
        if *ch == '\n' {
            seen += 1;
            if seen == line {
                return index + 1;
            }
        }
    }
    line_start(chars, chars.len())
}

pub fn first_non_blank(chars: &[char], pos: usize) -> usize {
    let start = line_start(chars, pos);
    let end = line_end(chars, pos);
    (start..end)
        .find(|index| !chars[*index].is_whitespace())
        .unwrap_or(start)
}

pub fn clamp_normal(chars: &[char], pos: usize) -> usize {
    let start = line_start(chars, pos);
    let end = line_end(chars, pos);
    if end == start {
        return start;
    }
    let pos = pos.clamp(start, end - 1);
    grapheme_start(chars, pos)
}

fn cluster_bounds(chars: &[char], pos: usize) -> Vec<usize> {
    use unicode_segmentation::UnicodeSegmentation;
    let start = line_start(chars, pos);
    let end = line_end(chars, pos);
    let line: String = chars[start..end].iter().collect();
    let mut bounds = Vec::new();
    let mut offset = start;
    for cluster in line.graphemes(true) {
        bounds.push(offset);
        offset += cluster.chars().count();
    }
    bounds.push(end);
    bounds
}

pub fn grapheme_start(chars: &[char], pos: usize) -> usize {
    let pos = pos.min(chars.len());
    cluster_bounds(chars, pos)
        .into_iter()
        .rev()
        .find(|bound| *bound <= pos)
        .unwrap_or(pos)
}

pub fn grapheme_next(chars: &[char], pos: usize, times: usize) -> usize {
    let pos = pos.min(chars.len());
    let bounds = cluster_bounds(chars, pos);
    let index = bounds.iter().rposition(|bound| *bound <= pos).unwrap_or(0);
    bounds[(index + times).min(bounds.len() - 1)]
}

pub fn grapheme_prev(chars: &[char], pos: usize, times: usize) -> usize {
    let pos = pos.min(chars.len());
    let bounds = cluster_bounds(chars, pos);
    match bounds.iter().position(|bound| *bound >= pos) {
        Some(index) => bounds[index.saturating_sub(times)],
        None => bounds[0],
    }
}

fn word_forward(chars: &[char], mut pos: usize, big: bool) -> usize {
    let len = chars.len();
    if pos >= len {
        return len;
    }
    let current = class(chars[pos], big);
    if current != 0 {
        while pos < len && class(chars[pos], big) == current {
            pos += 1;
        }
    }
    while pos < len && chars[pos].is_whitespace() {
        if chars[pos] == '\n' && pos + 1 < len && chars[pos + 1] == '\n' {
            return pos + 1;
        }
        pos += 1;
    }
    pos
}

fn word_end(chars: &[char], pos: usize, big: bool) -> usize {
    let len = chars.len();
    if len == 0 {
        return 0;
    }
    let mut pos = (pos + 1).min(len - 1);
    while pos + 1 < len && chars[pos].is_whitespace() {
        pos += 1;
    }
    let current = class(chars[pos], big);
    while pos + 1 < len && class(chars[pos + 1], big) == current && current != 0 {
        pos += 1;
    }
    pos
}

fn word_backward(chars: &[char], pos: usize, big: bool) -> usize {
    if pos == 0 || chars.is_empty() {
        return 0;
    }
    let mut pos = pos.min(chars.len()) - 1;
    while pos > 0 && chars[pos].is_whitespace() {
        if chars[pos] == '\n' && chars[pos - 1] == '\n' {
            return pos;
        }
        pos -= 1;
    }
    let current = class(chars[pos], big);
    while pos > 0 && class(chars[pos - 1], big) == current {
        pos -= 1;
    }
    pos
}

fn word_end_backward(chars: &[char], pos: usize, big: bool) -> usize {
    if chars.is_empty() {
        return 0;
    }
    let mut pos = pos.min(chars.len() - 1);
    let current = class(chars[pos], big);
    if current != 0 {
        while pos > 0 && class(chars[pos], big) == current {
            pos -= 1;
        }
    }
    while pos > 0 && chars[pos].is_whitespace() {
        pos -= 1;
    }
    pos
}

fn paragraph(chars: &[char], pos: usize, forward: bool) -> usize {
    let lines = line_count(chars);
    let mut line = line_index(chars, pos);
    let empty = |line: usize| {
        let start = nth_line_start(chars, line);
        line_end(chars, start) == start
    };
    if forward {
        while line + 1 < lines && empty(line) {
            line += 1;
        }
        while line + 1 < lines && !empty(line) {
            line += 1;
        }
        if line + 1 >= lines && !empty(line) {
            return line_end(chars, nth_line_start(chars, line));
        }
    } else {
        while line > 0 && empty(line) {
            line -= 1;
        }
        while line > 0 && !empty(line) {
            line -= 1;
        }
    }
    nth_line_start(chars, line)
}

fn sentence_starts(chars: &[char]) -> Vec<usize> {
    let len = chars.len();
    let mut starts = Vec::new();
    let mut index = 0;
    let mut fresh = true;
    while index < len {
        let ch = chars[index];
        if fresh {
            if ch == '\n' && index > 0 && chars[index - 1] == '\n' {
                starts.push(index);
                index += 1;
                continue;
            }
            if !ch.is_whitespace() {
                starts.push(index);
                fresh = false;
            }
            index += 1;
            continue;
        }
        if ch == '\n' && index + 1 < len && chars[index + 1] == '\n' {
            fresh = true;
            index += 1;
            continue;
        }
        if matches!(ch, '.' | '!' | '?') {
            let mut end = index + 1;
            while end < len && matches!(chars[end], ')' | ']' | '"' | '\'') {
                end += 1;
            }
            if end >= len || chars[end].is_whitespace() {
                fresh = true;
                index = end;
                continue;
            }
        }
        index += 1;
    }
    starts
}

fn sentence(chars: &[char], pos: usize, forward: bool) -> usize {
    let starts = sentence_starts(chars);
    if forward {
        starts
            .into_iter()
            .find(|start| *start > pos)
            .unwrap_or(chars.len())
    } else {
        starts
            .into_iter()
            .rev()
            .find(|start| *start < pos)
            .unwrap_or(0)
    }
}

fn sentence_range(chars: &[char], pos: usize, around: bool) -> Option<Range> {
    let starts = sentence_starts(chars);
    let start = starts.iter().rev().find(|start| **start <= pos).copied()?;
    let next = starts
        .iter()
        .find(|start| **start > pos)
        .copied()
        .unwrap_or(chars.len());
    let mut inner = next;
    while inner > start && chars[inner - 1].is_whitespace() {
        inner -= 1;
    }
    let end = if around && !chars[inner..next].contains(&'\n') {
        next
    } else if around {
        let mut end = inner;
        while end < next && chars[end].is_whitespace() && chars[end] != '\n' {
            end += 1;
        }
        end
    } else {
        inner
    };
    (end > start).then_some(Range {
        start,
        end,
        linewise: false,
    })
}

fn match_pair(chars: &[char], pos: usize) -> Option<usize> {
    const PAIRS: [(char, char); 3] = [('(', ')'), ('[', ']'), ('{', '}')];
    let end = line_end(chars, pos);
    let start = (pos..end).find(|index| {
        PAIRS
            .iter()
            .any(|(open, close)| chars[*index] == *open || chars[*index] == *close)
    })?;
    let ch = chars[start];
    for (open, close) in PAIRS {
        if ch == open {
            let mut depth = 0usize;
            for (index, current) in chars.iter().enumerate().skip(start) {
                if *current == open {
                    depth += 1;
                } else if *current == close {
                    depth -= 1;
                    if depth == 0 {
                        return Some(index);
                    }
                }
            }
            return None;
        }
        if ch == close {
            let mut depth = 0usize;
            for index in (0..=start).rev() {
                if chars[index] == close {
                    depth += 1;
                } else if chars[index] == open {
                    depth -= 1;
                    if depth == 0 {
                        return Some(index);
                    }
                }
            }
            return None;
        }
    }
    None
}

fn find(chars: &[char], pos: usize, forward: bool, till: bool, ch: char) -> Option<usize> {
    let start = line_start(chars, pos);
    let end = line_end(chars, pos);
    if forward {
        let from = pos + 1 + usize::from(till);
        let found = (from.min(end)..end).find(|index| chars[*index] == ch)?;
        Some(if till { found - 1 } else { found })
    } else {
        let to = pos.saturating_sub(usize::from(till));
        let found = (start..to).rev().find(|index| chars[*index] == ch)?;
        Some(if till { found + 1 } else { found })
    }
}

fn vertical(chars: &[char], pos: usize, down: bool, count: usize) -> usize {
    let column = pos - line_start(chars, pos);
    let line = line_index(chars, pos);
    let lines = line_count(chars);
    let target = if down {
        (line + count).min(lines - 1)
    } else {
        line.saturating_sub(count)
    };
    let start = nth_line_start(chars, target);
    let end = line_end(chars, start);
    (start + column).min(end)
}

pub fn motion(
    chars: &[char],
    pos: usize,
    motion: Motion,
    count: Option<usize>,
    last_find: &mut LastFind,
) -> Option<usize> {
    let times = count.unwrap_or(1).max(1);
    let repeat = |step: &dyn Fn(usize) -> usize| {
        let mut current = pos;
        for _ in 0..times {
            current = step(current);
        }
        current
    };
    Some(match motion {
        Motion::Left => grapheme_prev(chars, pos, times),
        Motion::Right => grapheme_next(chars, pos, times),
        Motion::Up => vertical(chars, pos, false, times),
        Motion::Down => vertical(chars, pos, true, times),
        Motion::WordForward => repeat(&|p| word_forward(chars, p, false)),
        Motion::BigWordForward => repeat(&|p| word_forward(chars, p, true)),
        Motion::WordEnd => repeat(&|p| word_end(chars, p, false)),
        Motion::BigWordEnd => repeat(&|p| word_end(chars, p, true)),
        Motion::WordBackward => repeat(&|p| word_backward(chars, p, false)),
        Motion::BigWordBackward => repeat(&|p| word_backward(chars, p, true)),
        Motion::WordEndBackward => repeat(&|p| word_end_backward(chars, p, false)),
        Motion::BigWordEndBackward => repeat(&|p| word_end_backward(chars, p, true)),
        Motion::LineStart => line_start(chars, pos),
        Motion::FirstNonBlank => first_non_blank(chars, pos),
        Motion::LineEnd => {
            let target = vertical(chars, pos, true, times - 1);
            line_end(chars, target)
                .saturating_sub(1)
                .max(line_start(chars, target))
        }
        Motion::LastNonBlank => {
            let start = line_start(chars, pos);
            let end = line_end(chars, pos);
            (start..end)
                .rev()
                .find(|index| !chars[*index].is_whitespace())
                .unwrap_or(start)
        }
        Motion::FirstLine => {
            let line = count.map_or(0, |count| count.saturating_sub(1));
            first_non_blank(chars, nth_line_start(chars, line))
        }
        Motion::LastLine => {
            let line = count.map_or(line_count(chars) - 1, |count| count.saturating_sub(1));
            first_non_blank(chars, nth_line_start(chars, line))
        }
        Motion::ParagraphForward => repeat(&|p| paragraph(chars, p, true)),
        Motion::ParagraphBackward => repeat(&|p| paragraph(chars, p, false)),
        Motion::SentenceForward => repeat(&|p| sentence(chars, p, true)),
        Motion::SentenceBackward => repeat(&|p| sentence(chars, p, false)),
        Motion::MatchPair => match count {
            Some(percent) => {
                let lines = line_count(chars);
                let line = (percent.min(100) * lines).div_ceil(100).max(1) - 1;
                first_non_blank(chars, nth_line_start(chars, line))
            }
            None => match_pair(chars, pos)?,
        },
        Motion::Find { forward, till, ch } => {
            *last_find = LastFind {
                forward,
                till,
                ch: Some(ch),
            };
            let mut current = pos;
            for _ in 0..times {
                current = find(chars, current, forward, till, ch)?;
            }
            current
        }
        Motion::RepeatFind { reverse } => {
            let ch = last_find.ch?;
            let forward = last_find.forward != reverse;
            let mut current = pos;
            for _ in 0..times {
                let next = find(chars, current, forward, last_find.till, ch);
                let next = match next {
                    Some(next) if next == current && last_find.till => {
                        let skip = if forward {
                            current + 1
                        } else {
                            current.saturating_sub(1)
                        };
                        find(chars, skip, forward, true, ch)?
                    }
                    Some(next) => next,
                    None => return None,
                };
                current = next;
            }
            current
        }
    })
}

fn full_lines(chars: &[char], first: usize, last: usize) -> Range {
    let start = nth_line_start(chars, first);
    let end_line_start = nth_line_start(chars, last);
    let end = line_end(chars, end_line_start);
    if end < chars.len() {
        Range {
            start,
            end: end + 1,
            linewise: true,
        }
    } else {
        Range {
            start: start.saturating_sub(usize::from(start > 0)),
            end,
            linewise: true,
        }
    }
}

pub fn line_range(chars: &[char], pos: usize, count: Option<usize>) -> Range {
    let first = line_index(chars, pos);
    let last = (first + count.unwrap_or(1).max(1) - 1).min(line_count(chars) - 1);
    full_lines(chars, first, last)
}

pub fn lines_between(chars: &[char], a: usize, b: usize) -> Range {
    let first = line_index(chars, a.min(b));
    let last = line_index(chars, a.max(b));
    full_lines(chars, first, last)
}

pub fn motion_range(
    chars: &[char],
    pos: usize,
    target: Motion,
    count: Option<usize>,
    change: bool,
    last_find: &mut LastFind,
) -> Option<Range> {
    let on_word = chars.get(pos).is_some_and(|ch| !ch.is_whitespace());
    let target = match target {
        Motion::WordForward if change && on_word => Motion::WordEnd,
        Motion::BigWordForward if change && on_word => Motion::BigWordEnd,
        other => other,
    };
    let substituted = change && on_word && matches!(target, Motion::WordEnd | Motion::BigWordEnd);
    let from = if substituted && pos > 0 { pos - 1 } else { pos };
    let dest = if from != pos {
        let mut found = motion(chars, from, target, count, last_find)?;
        if found < pos {
            found = motion(chars, pos, target, count, last_find)?;
        }
        found
    } else {
        motion(chars, pos, target, count, last_find)?
    };
    if target.linewise() || (target == Motion::MatchPair && count.is_some()) {
        return Some(lines_between(chars, pos, dest));
    }
    let (mut start, mut end) = (pos.min(dest), pos.max(dest));
    if target.inclusive() {
        end = grapheme_next(chars, end, 1).max(end + 1).min(chars.len());
    }
    if matches!(target, Motion::WordForward | Motion::BigWordForward) {
        let limit = line_end(chars, start);
        if end > limit && limit > start {
            end = limit;
        }
    }
    if start == end && !target.inclusive() {
        return None;
    }
    start = start.min(chars.len());
    Some(Range {
        start,
        end,
        linewise: false,
    })
}

pub fn object_range(chars: &[char], pos: usize, object: Object, around: bool) -> Option<Range> {
    if chars.is_empty() {
        return None;
    }
    let pos = pos.min(chars.len() - 1);
    match object {
        Object::Word | Object::BigWord => {
            let big = object == Object::BigWord;
            let current = class(chars[pos], big);
            let same = |index: usize| class(chars[index], big) == current && chars[index] != '\n';
            let mut start = pos;
            while start > 0 && same(start - 1) {
                start -= 1;
            }
            let mut end = pos + 1;
            while end < chars.len() && same(end) {
                end += 1;
            }
            if around {
                let trailing = end;
                while end < chars.len() && chars[end].is_whitespace() && chars[end] != '\n' {
                    end += 1;
                }
                if end == trailing {
                    while start > 0 && chars[start - 1].is_whitespace() && chars[start - 1] != '\n'
                    {
                        start -= 1;
                    }
                }
            }
            Some(Range {
                start,
                end,
                linewise: false,
            })
        }
        Object::Sentence => sentence_range(chars, pos, around),
        Object::Paragraph => {
            let lines = line_count(chars);
            let empty = |line: usize| {
                let start = nth_line_start(chars, line);
                line_end(chars, start) == start
            };
            let line = line_index(chars, pos);
            let kind = empty(line);
            let mut first = line;
            while first > 0 && empty(first - 1) == kind {
                first -= 1;
            }
            let mut last = line;
            while last + 1 < lines && empty(last + 1) == kind {
                last += 1;
            }
            if around {
                while last + 1 < lines && empty(last + 1) != kind {
                    last += 1;
                    if !empty(last) {
                        last -= 1;
                        break;
                    }
                }
            }
            Some(full_lines(chars, first, last))
        }
        Object::Quote(quote) => {
            let start_line = line_start(chars, pos);
            let end_line = line_end(chars, pos);
            let quotes: Vec<usize> = (start_line..end_line)
                .filter(|index| chars[*index] == quote)
                .collect();
            let pair = quotes
                .chunks(2)
                .filter(|pair| pair.len() == 2)
                .find(|pair| pos >= pair[0] && pos <= pair[1])
                .or_else(|| {
                    quotes
                        .chunks(2)
                        .filter(|pair| pair.len() == 2)
                        .find(|pair| pair[0] > pos)
                })?;
            let (open, close) = (pair[0], pair[1]);
            Some(if around {
                Range {
                    start: open,
                    end: close + 1,
                    linewise: false,
                }
            } else {
                Range {
                    start: open + 1,
                    end: close,
                    linewise: false,
                }
            })
        }
        Object::Pair(open, close) => {
            let mut depth = 0usize;
            let mut start = None;
            for index in (0..=pos).rev() {
                if chars[index] == close && index != pos {
                    depth += 1;
                } else if chars[index] == open {
                    if depth == 0 {
                        start = Some(index);
                        break;
                    }
                    depth -= 1;
                }
            }
            let start = start?;
            let mut depth = 0usize;
            let mut end = None;
            for (index, ch) in chars.iter().enumerate().skip(start + 1) {
                if *ch == open {
                    depth += 1;
                } else if *ch == close {
                    if depth == 0 {
                        end = Some(index);
                        break;
                    }
                    depth -= 1;
                }
            }
            let end = end?;
            Some(if around {
                Range {
                    start,
                    end: end + 1,
                    linewise: false,
                }
            } else {
                Range {
                    start: start + 1,
                    end,
                    linewise: false,
                }
            })
        }
    }
}

pub fn target_range(
    chars: &[char],
    pos: usize,
    target: Target,
    count: Option<usize>,
    change: bool,
    last_find: &mut LastFind,
) -> Option<Range> {
    match target {
        Target::Line => Some(line_range(chars, pos, count)),
        Target::Motion(motion) => motion_range(chars, pos, motion, count, change, last_find),
        Target::Inner(object) => object_range(chars, pos, object, false),
        Target::Around(object) => object_range(chars, pos, object, true),
    }
}

pub fn transform(text: &str, op: Operator) -> String {
    match op {
        Operator::Lower => text.to_lowercase(),
        Operator::Upper => text.to_uppercase(),
        Operator::ToggleCase => text
            .chars()
            .map(|ch| {
                if ch.is_uppercase() {
                    ch.to_lowercase().collect::<String>()
                } else {
                    ch.to_uppercase().collect::<String>()
                }
            })
            .collect(),
        Operator::Indent => text
            .split('\n')
            .map(|line| {
                if line.is_empty() {
                    String::new()
                } else {
                    format!("  {line}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Operator::Outdent => text
            .split('\n')
            .map(|line| {
                let strip = line.chars().take(2).take_while(|ch| *ch == ' ').count();
                line[strip..].to_owned()
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Operator::Delete | Operator::Change | Operator::Yank => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(text: &str) -> Vec<char> {
        text.chars().collect()
    }

    fn go(text: &str, pos: usize, target: Motion) -> usize {
        motion(&chars(text), pos, target, None, &mut LastFind::default()).unwrap()
    }

    #[test]
    fn word_motions_follow_vim_classes() {
        let text = "foo.bar  baz\nqux";
        assert_eq!(go(text, 0, Motion::WordForward), 3);
        assert_eq!(go(text, 3, Motion::WordForward), 4);
        assert_eq!(go(text, 0, Motion::BigWordForward), 9);
        assert_eq!(go(text, 9, Motion::WordForward), 13);
        assert_eq!(go(text, 0, Motion::WordEnd), 2);
        assert_eq!(go(text, 13, Motion::WordBackward), 9);
        assert_eq!(go(text, 9, Motion::BigWordBackward), 0);
        assert_eq!(go(text, 4, Motion::LineEnd), 11);
        assert_eq!(go(text, 14, Motion::Up), 1);
    }

    #[test]
    fn find_and_repeat() {
        let text = "a,b,c,d";
        let mut last = LastFind::default();
        let found = motion(
            &chars(text),
            0,
            Motion::Find {
                forward: true,
                till: false,
                ch: ',',
            },
            None,
            &mut last,
        );
        assert_eq!(found, Some(1));
        let again = motion(
            &chars(text),
            1,
            Motion::RepeatFind { reverse: false },
            Some(2),
            &mut last,
        );
        assert_eq!(again, Some(5));
    }

    #[test]
    fn operator_ranges() {
        let text = chars("one two three\nfour");
        let mut last = LastFind::default();
        let dw = motion_range(&text, 0, Motion::WordForward, None, false, &mut last).unwrap();
        assert_eq!((dw.start, dw.end), (0, 4));
        let cw = motion_range(&text, 0, Motion::WordForward, None, true, &mut last).unwrap();
        assert_eq!((cw.start, cw.end), (0, 3));
        let d_end = motion_range(&text, 8, Motion::WordForward, None, false, &mut last).unwrap();
        assert_eq!((d_end.start, d_end.end), (8, 13));
        let dd = line_range(&text, 3, None);
        assert_eq!((dd.start, dd.end, dd.linewise), (0, 14, true));
        let last_line = line_range(&text, 15, None);
        assert_eq!((last_line.start, last_line.end), (13, 18));
        let iw = object_range(&text, 5, Object::Word, false).unwrap();
        assert_eq!((iw.start, iw.end), (4, 7));
        let aw = object_range(&text, 5, Object::Word, true).unwrap();
        assert_eq!((aw.start, aw.end), (4, 8));
    }

    #[test]
    fn pair_and_quote_objects() {
        let text = chars("call(a, \"b c\")");
        let inner = object_range(&text, 6, Object::Pair('(', ')'), false).unwrap();
        assert_eq!((inner.start, inner.end), (5, 13));
        let quote = object_range(&text, 10, Object::Quote('"'), false).unwrap();
        assert_eq!((quote.start, quote.end), (9, 12));
        assert_eq!(go("f(x)", 1, Motion::MatchPair), 3);
    }

    #[test]
    fn transforms() {
        assert_eq!(transform("aB", Operator::ToggleCase), "Ab");
        assert_eq!(transform("a\n\nb", Operator::Indent), "  a\n\n  b");
        assert_eq!(transform("   a\n b", Operator::Outdent), " a\nb");
    }

    fn counted(text: &str, pos: usize, target: Motion, count: usize) -> usize {
        motion(
            &chars(text),
            pos,
            target,
            Some(count),
            &mut LastFind::default(),
        )
        .unwrap()
    }

    fn span(range: Range) -> (usize, usize) {
        (range.start, range.end)
    }

    #[test]
    fn word_motions_on_cyrillic_cjk_and_emoji() {
        let text = "привет, мир 世界ok 😀😀 end";
        assert_eq!(go(text, 0, Motion::WordForward), 6);
        assert_eq!(go(text, 6, Motion::WordForward), 8);
        assert_eq!(go(text, 8, Motion::WordForward), 12);
        assert_eq!(go(text, 12, Motion::WordForward), 17);
        assert_eq!(go(text, 17, Motion::WordForward), 20);
        assert_eq!(go(text, 0, Motion::WordEnd), 5);
        assert_eq!(go(text, 12, Motion::WordEnd), 15);
        assert_eq!(go(text, 20, Motion::WordBackward), 17);
        assert_eq!(go(text, 17, Motion::WordBackward), 12);
        assert_eq!(go(text, 0, Motion::BigWordForward), 8);
        assert_eq!(counted(text, 0, Motion::WordForward, 3), 12);
        assert_eq!(go(text, 12, Motion::WordEndBackward), 10);
    }

    #[test]
    fn left_and_right_step_over_grapheme_clusters() {
        let text = "e\u{301}x👨\u{200d}👩\u{200d}👧y";
        let line = chars(text);
        assert_eq!(line.len(), 9);
        assert_eq!(go(text, 0, Motion::Right), 2);
        assert_eq!(go(text, 2, Motion::Right), 3);
        assert_eq!(go(text, 3, Motion::Right), 8);
        assert_eq!(go(text, 8, Motion::Left), 3);
        assert_eq!(go(text, 3, Motion::Left), 2);
        assert_eq!(go(text, 2, Motion::Left), 0);
        assert_eq!(counted(text, 0, Motion::Right, 3), 8);
        assert_eq!(clamp_normal(&line, 1), 0);
        assert_eq!(clamp_normal(&line, 5), 3);
        assert_eq!(clamp_normal(&line, 99), 8);
        assert_eq!(grapheme_next(&line, 8, 1), 9);
        assert_eq!(grapheme_prev(&line, 9, 1), 8);
        assert_eq!(grapheme_start(&line, 7), 3);
    }

    #[test]
    fn inclusive_motions_cover_the_whole_last_cluster() {
        let text = chars("cafe\u{301} bar");
        let mut last = LastFind::default();
        let range = motion_range(&text, 0, Motion::WordEnd, None, false, &mut last).unwrap();
        assert_eq!(span(range), (0, 5));
        let range = motion_range(&text, 0, Motion::LineEnd, None, false, &mut last).unwrap();
        assert_eq!(span(range), (0, 9));
    }

    #[test]
    fn line_motions_keep_columns_in_characters() {
        let text = "日本語の文\nabc\nдлинная строка";
        assert_eq!(go(text, 3, Motion::Down), 9);
        assert_eq!(go(text, 9, Motion::Down), 13);
        assert_eq!(go(text, 13, Motion::Up), 9);
        assert_eq!(go(text, 4, Motion::LineEnd), 4);
        assert_eq!(go(text, 0, Motion::LastLine), 10);
        assert_eq!(counted(text, 20, Motion::FirstLine, 2), 6);
    }

    #[test]
    fn find_works_on_multibyte_characters() {
        let text = "ёж, ёлка, ёрш";
        let mut last = LastFind::default();
        let line = chars(text);
        let found = motion(
            &line,
            0,
            Motion::Find {
                forward: true,
                till: false,
                ch: 'ё',
            },
            None,
            &mut last,
        );
        assert_eq!(found, Some(4));
        let again = motion(
            &line,
            4,
            Motion::RepeatFind { reverse: false },
            None,
            &mut last,
        );
        assert_eq!(again, Some(10));
        let back = motion(
            &line,
            10,
            Motion::RepeatFind { reverse: true },
            None,
            &mut last,
        );
        assert_eq!(back, Some(4));
        let till = motion(
            &line,
            0,
            Motion::Find {
                forward: true,
                till: true,
                ch: ',',
            },
            None,
            &mut last,
        );
        assert_eq!(till, Some(1));
    }

    #[test]
    fn chip_sentinels_form_their_own_word() {
        let mut line = chars("see @src/main.rs, ok");
        for ch in line.iter_mut().take(16).skip(4) {
            *ch = CHIP;
        }
        let text: String = line.iter().collect();
        assert_eq!(go(&text, 0, Motion::WordForward), 4);
        assert_eq!(go(&text, 4, Motion::WordForward), 16);
        assert_eq!(go(&text, 4, Motion::BigWordForward), 16);
        assert_eq!(go(&text, 18, Motion::WordBackward), 16);
        assert_eq!(go(&text, 16, Motion::WordBackward), 4);
        let iw = object_range(&line, 8, Object::Word, false).unwrap();
        assert_eq!(span(iw), (4, 16));
    }

    #[test]
    fn sentence_motions_and_objects() {
        let text = "One two. Three four! Five?\n\nSix.";
        assert_eq!(go(text, 0, Motion::SentenceForward), 9);
        assert_eq!(go(text, 9, Motion::SentenceForward), 21);
        assert_eq!(go(text, 21, Motion::SentenceForward), 27);
        assert_eq!(go(text, 27, Motion::SentenceForward), 28);
        assert_eq!(go(text, 28, Motion::SentenceForward), 32);
        assert_eq!(go(text, 12, Motion::SentenceBackward), 9);
        assert_eq!(go(text, 9, Motion::SentenceBackward), 0);
        let line = chars(text);
        let inner = object_range(&line, 12, Object::Sentence, false).unwrap();
        assert_eq!(span(inner), (9, 20));
        let around = object_range(&line, 12, Object::Sentence, true).unwrap();
        assert_eq!(span(around), (9, 21));
        let last = object_range(&line, 22, Object::Sentence, true).unwrap();
        assert_eq!(span(last), (21, 26));
        let quoted = chars("He said \"go.\" Then left.");
        assert_eq!(sentence(&quoted, 0, true), 14);
    }

    #[test]
    fn percent_with_a_count_goes_to_a_line() {
        let text = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj";
        assert_eq!(counted(text, 0, Motion::MatchPair, 50), 8);
        assert_eq!(counted(text, 0, Motion::MatchPair, 100), 18);
        assert_eq!(counted(text, 18, Motion::MatchPair, 1), 0);
        let mut last = LastFind::default();
        let range = motion_range(
            &chars(text),
            0,
            Motion::MatchPair,
            Some(20),
            false,
            &mut last,
        )
        .unwrap();
        assert!(range.linewise);
        assert_eq!(span(range), (0, 4));
    }

    #[test]
    fn objects_on_unicode_text() {
        let line = chars("«x» (привет мир) 'ü'");
        let paren = object_range(&line, 7, Object::Pair('(', ')'), false).unwrap();
        assert_eq!(span(paren), (5, 15));
        let around = object_range(&line, 7, Object::Pair('(', ')'), true).unwrap();
        assert_eq!(span(around), (4, 16));
        let word = object_range(&line, 7, Object::Word, true).unwrap();
        assert_eq!(span(word), (5, 12));
        let quote = object_range(&line, 18, Object::Quote('\''), false).unwrap();
        assert_eq!(span(quote), (18, 19));
        let big = object_range(&line, 1, Object::BigWord, false).unwrap();
        assert_eq!(span(big), (0, 3));
    }

    #[test]
    fn case_transforms_handle_non_ascii() {
        assert_eq!(transform("Ёлка straße", Operator::Upper), "ЁЛКА STRASSE");
        assert_eq!(transform("ЁЛКА", Operator::Lower), "ёлка");
        assert_eq!(transform("Привет", Operator::ToggleCase), "пРИВЕТ");
    }
}
