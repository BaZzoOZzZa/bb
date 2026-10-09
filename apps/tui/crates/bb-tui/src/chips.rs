use bb_wire::prompt::{PromptMentionResource, PromptTextMention};

pub fn char_len(text: &str) -> usize {
    text.chars().count()
}

pub fn char_to_utf16(text: &str, char_offset: usize) -> usize {
    text.chars().take(char_offset).map(char::len_utf16).sum()
}

pub fn utf16_to_char(text: &str, utf16: usize) -> Option<usize> {
    let mut units = 0;
    for (index, ch) in text.chars().enumerate() {
        if units == utf16 {
            return Some(index);
        }
        if units > utf16 {
            return None;
        }
        units += ch.len_utf16();
    }
    (units == utf16).then(|| char_len(text))
}

pub fn char_to_byte(text: &str, char_offset: usize) -> usize {
    text.char_indices()
        .nth(char_offset)
        .map_or(text.len(), |(index, _)| index)
}

pub fn char_to_row_col(text: &str, char_offset: usize) -> (usize, usize) {
    let mut row = 0;
    let mut col = 0;
    for ch in text.chars().take(char_offset) {
        if ch == '\n' {
            row += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    (row, col)
}

pub fn row_col_to_char(lines: &[String], row: usize, col: usize) -> usize {
    let before: usize = lines.iter().take(row).map(|line| char_len(line) + 1).sum();
    let line_len = lines.get(row).map_or(0, |line| char_len(line));
    before + col.min(line_len)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CharRange {
    pub start: usize,
    pub end: usize,
}

impl CharRange {
    pub fn contains_strictly(self, offset: usize) -> bool {
        self.start < offset && offset < self.end
    }
}

pub fn mention_char_range(text: &str, mention: &PromptTextMention) -> Option<CharRange> {
    let start = utf16_to_char(text, mention.start as usize)?;
    let end = utf16_to_char(text, mention.end as usize)?;
    (start < end).then_some(CharRange { start, end })
}

pub fn mention_char_ranges(text: &str, mentions: &[PromptTextMention]) -> Vec<CharRange> {
    mentions
        .iter()
        .filter_map(|mention| mention_char_range(text, mention))
        .collect()
}

fn utf16_mention(
    text: &str,
    range: CharRange,
    resource: PromptMentionResource,
) -> PromptTextMention {
    PromptTextMention {
        start: char_to_utf16(text, range.start) as u32,
        end: char_to_utf16(text, range.end) as u32,
        resource,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edit {
    pub start: usize,
    pub old_end: usize,
    pub new_end: usize,
}

pub fn diff_edit(old: &[char], new: &[char], cursor: Option<usize>) -> Option<Edit> {
    if old == new {
        return None;
    }
    let prefix_max = old
        .iter()
        .zip(new.iter())
        .take_while(|(left, right)| left == right)
        .count();
    let suffix_max = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take_while(|(left, right)| left == right)
        .count();
    let common = (prefix_max + suffix_max).min(old.len()).min(new.len());
    let lowest_suffix = common.saturating_sub(prefix_max);
    let highest_suffix = suffix_max.min(common);
    let wanted = cursor.map_or(lowest_suffix, |cursor| new.len().saturating_sub(cursor));
    let suffix = wanted.clamp(lowest_suffix, highest_suffix.max(lowest_suffix));
    let prefix = common - suffix;
    Some(Edit {
        start: prefix,
        old_end: old.len() - suffix,
        new_end: new.len() - suffix,
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct Reconciled {
    pub text: String,
    pub mentions: Vec<PromptTextMention>,
    pub cursor: Option<usize>,
    pub removed_leftovers: bool,
}

pub fn reconcile(
    old_text: &str,
    new_text: &str,
    mentions: &[PromptTextMention],
    cursor: Option<usize>,
) -> Reconciled {
    let old: Vec<char> = old_text.chars().collect();
    let new: Vec<char> = new_text.chars().collect();
    let Some(edit) = diff_edit(&old, &new, cursor) else {
        return Reconciled {
            text: new_text.to_owned(),
            mentions: mentions.to_vec(),
            cursor,
            removed_leftovers: false,
        };
    };
    let delta = edit.new_end as isize - edit.old_end as isize;
    let shift = |offset: usize| (offset as isize + delta) as usize;
    let mut kept: Vec<(CharRange, PromptMentionResource)> = Vec::new();
    let mut removals: Vec<CharRange> = Vec::new();
    for mention in mentions {
        let Some(range) = mention_char_range(old_text, mention) else {
            continue;
        };
        if range.end <= edit.start {
            kept.push((range, mention.resource.clone()));
        } else if range.start >= edit.old_end {
            kept.push((
                CharRange {
                    start: shift(range.start),
                    end: shift(range.end),
                },
                mention.resource.clone(),
            ));
        } else {
            if range.start < edit.start {
                removals.push(CharRange {
                    start: range.start,
                    end: edit.start,
                });
            }
            if edit.old_end < range.end {
                removals.push(CharRange {
                    start: shift(edit.old_end),
                    end: shift(range.end),
                });
            }
        }
    }
    removals.sort_by_key(|range| range.start);
    let removed_before = |offset: usize| -> usize {
        removals
            .iter()
            .map(|range| {
                if offset >= range.end {
                    range.end - range.start
                } else {
                    offset.saturating_sub(range.start)
                }
            })
            .sum()
    };
    let text: String = if removals.is_empty() {
        new_text.to_owned()
    } else {
        new.iter()
            .enumerate()
            .filter(|(index, _)| {
                !removals
                    .iter()
                    .any(|range| range.start <= *index && *index < range.end)
            })
            .map(|(_, ch)| *ch)
            .collect()
    };
    let mentions = kept
        .into_iter()
        .map(|(range, resource)| {
            let range = CharRange {
                start: range.start - removed_before(range.start),
                end: range.end - removed_before(range.end),
            };
            utf16_mention(&text, range, resource)
        })
        .collect();
    Reconciled {
        cursor: cursor.map(|cursor| cursor - removed_before(cursor)),
        removed_leftovers: !removals.is_empty(),
        text,
        mentions,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Inserted {
    pub text: String,
    pub mentions: Vec<PromptTextMention>,
    pub cursor: usize,
}

pub fn insert_mention(
    text: &str,
    mentions: &[PromptTextMention],
    replace: CharRange,
    serialized: &str,
    resource: PromptMentionResource,
    trailing: &str,
) -> Inserted {
    let from = char_to_byte(text, replace.start);
    let to = char_to_byte(text, replace.end.max(replace.start));
    let mut next = String::with_capacity(text.len() + serialized.len() + trailing.len());
    next.push_str(&text[..from]);
    next.push_str(serialized);
    next.push_str(trailing);
    next.push_str(&text[to..]);
    let inserted = char_len(serialized) + char_len(trailing);
    let removed = replace.end.saturating_sub(replace.start);
    let mut ranges: Vec<(CharRange, PromptMentionResource)> = Vec::new();
    for mention in mentions {
        let Some(range) = mention_char_range(text, mention) else {
            continue;
        };
        if range.end <= replace.start {
            ranges.push((range, mention.resource.clone()));
        } else if range.start >= replace.end {
            ranges.push((
                CharRange {
                    start: range.start + inserted - removed,
                    end: range.end + inserted - removed,
                },
                mention.resource.clone(),
            ));
        }
    }
    ranges.push((
        CharRange {
            start: replace.start,
            end: replace.start + char_len(serialized),
        },
        resource,
    ));
    ranges.sort_by_key(|(range, _)| range.start);
    let mentions = ranges
        .into_iter()
        .map(|(range, resource)| utf16_mention(&next, range, resource))
        .collect();
    Inserted {
        cursor: replace.start + inserted,
        text: next,
        mentions,
    }
}

pub fn snap_cursor(
    text: &str,
    mentions: &[PromptTextMention],
    cursor: usize,
    backward: bool,
) -> usize {
    mention_char_ranges(text, mentions)
        .into_iter()
        .find(|range| range.contains_strictly(cursor))
        .map_or(
            cursor,
            |range| if backward { range.start } else { range.end },
        )
}

pub fn trailing_text_after(text: &str, char_offset: usize) -> &'static str {
    match text.chars().nth(char_offset) {
        Some(ch) if ch.is_whitespace() => "",
        _ => " ",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bb_wire::prompt::{PathEntryKind, PromptMentionPathSource};

    fn path(name: &str) -> PromptMentionResource {
        PromptMentionResource::Path {
            source: PromptMentionPathSource::Workspace,
            entry_kind: PathEntryKind::File,
            path: name.into(),
            label: name.into(),
        }
    }

    fn mention(start: u32, end: u32, name: &str) -> PromptTextMention {
        PromptTextMention {
            start,
            end,
            resource: path(name),
        }
    }

    #[test]
    fn utf16_offsets_round_trip_through_chars() {
        let text = "a😀b👨‍👩‍👧c";
        assert_eq!(char_to_utf16(text, 1), 1);
        assert_eq!(char_to_utf16(text, 2), 3);
        assert_eq!(utf16_to_char(text, 3), Some(2));
        assert_eq!(utf16_to_char(text, 2), None);
        assert_eq!(utf16_to_char(text, 12), Some(8));
        assert_eq!(utf16_to_char(text, 13), Some(char_len(text)));
        assert_eq!(utf16_to_char(text, 14), None);
        assert_eq!(utf16_to_char(text, 5), None);
        assert_eq!(utf16_to_char(text, 6), Some(4));
    }

    #[test]
    fn diff_prefers_the_region_ending_at_the_cursor() {
        let old: Vec<char> = "a@a".chars().collect();
        let new: Vec<char> = "a@a@a".chars().collect();
        assert_eq!(
            diff_edit(&old, &new, Some(3)),
            Some(Edit {
                start: 1,
                old_end: 1,
                new_end: 3
            })
        );
        assert_eq!(
            diff_edit(&old, &new, None),
            Some(Edit {
                start: 3,
                old_end: 3,
                new_end: 5
            })
        );
    }

    #[test]
    fn typing_before_a_chip_shifts_it_by_utf16_units() {
        let old = "@a.rs x";
        let mentions = vec![mention(0, 5, "a.rs")];
        let result = reconcile(old, "😀@a.rs x", &mentions, Some(1));
        assert_eq!(result.mentions, vec![mention(2, 7, "a.rs")]);
        assert!(!result.removed_leftovers);
    }

    #[test]
    fn backspace_at_chip_end_removes_the_whole_chip() {
        let old = "see @файл.rs now";
        let mentions = vec![mention(4, 12, "файл.rs")];
        let result = reconcile(old, "see @файл.r now", &mentions, Some(11));
        assert_eq!(result.text, "see  now");
        assert!(result.mentions.is_empty());
        assert_eq!(result.cursor, Some(4));
        assert!(result.removed_leftovers);
    }

    #[test]
    fn deleting_a_word_across_a_chip_removes_its_rest() {
        let old = "x @a/b.rs y";
        let mentions = vec![mention(2, 9, "a/b.rs")];
        let result = reconcile(old, "x @a/y", &mentions, Some(5));
        assert_eq!(result.text, "x y");
        assert!(result.mentions.is_empty());
    }

    #[test]
    fn inserting_mention_replaces_trigger_and_keeps_others() {
        let text = "日本 @a.rs and @q";
        let mentions = vec![mention(3, 8, "a.rs")];
        let inserted = insert_mention(
            text,
            &mentions,
            CharRange { start: 13, end: 15 },
            "@src/😀.rs",
            path("src/😀.rs"),
            " ",
        );
        assert_eq!(inserted.text, "日本 @a.rs and @src/😀.rs ");
        assert_eq!(inserted.mentions.len(), 2);
        assert_eq!(inserted.mentions[1].start, 13);
        assert_eq!(inserted.mentions[1].end, 23);
        assert_eq!(inserted.cursor, char_len("日本 @a.rs and @src/😀.rs "));
    }

    #[test]
    fn cursor_snaps_out_of_chips() {
        let text = "a @b.rs c";
        let mentions = vec![mention(2, 7, "b.rs")];
        assert_eq!(snap_cursor(text, &mentions, 4, false), 7);
        assert_eq!(snap_cursor(text, &mentions, 4, true), 2);
        assert_eq!(snap_cursor(text, &mentions, 2, false), 2);
        assert_eq!(snap_cursor(text, &mentions, 7, true), 7);
    }
}
