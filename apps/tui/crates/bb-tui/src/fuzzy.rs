use std::cmp::Ordering;
use std::collections::HashMap;

use nucleo_matcher::chars::{normalize, to_lower_case};
use nucleo_matcher::{Config, Matcher, Utf32Str};

pub const QUERY_MAX_LENGTH: usize = 256;

const STRUCTURED_QUERY_SEGMENT_MAX_COUNT: usize = 8;
const IMPLICIT_DIRECTORY_QUERY_MIN_LENGTH: usize = 2;

const RANK_UNIT: i64 = 1_000_000;
const DIRECTORY_SEGMENT_EXACT: i64 = 30_000;
const DIRECTORY_SEGMENT_PREFIX: i64 = 25_000;
const ROOT_DIRECTORY_SEGMENT: i64 = 5_000;
const CONSECUTIVE_SEGMENT: i64 = 5_000;
const LEAF_SEGMENT: i64 = 50_000;

const BASE_NAME_CONTAINS: i64 = 10_000;
const BASE_NAME_SUBSEQUENCE: i64 = 5_000;
const PATH_CONTAINS: i64 = 1_000;
const REPEATED_PATH_HIT: i64 = 500;

const TEXT_EXACT: i64 = 20_000;
const TEXT_PREFIX: i64 = 15_000;
const TEXT_CONTAINS: i64 = 10_000;
const TEXT_SUBSEQUENCE: i64 = 5_000;
const PRIMARY_TEXT_SCORE: i64 = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum PathIntent {
    PlainFzf = 0,
    DirectorySegmentPrefix = 1,
    StructuredPath = 2,
    ExactDirectoryPrefix = 3,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuzzyMatch {
    pub index: usize,
    pub score: i64,
    pub positions: Vec<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Casing {
    Smart,
    Insensitive,
}

#[derive(Debug, Clone)]
struct Hit {
    score: i64,
    start: usize,
    positions: Vec<usize>,
}

struct Fzf {
    matcher: Matcher,
    haystack: Vec<char>,
    needle: Vec<char>,
    indices: Vec<u32>,
}

impl Fzf {
    fn new() -> Self {
        Self {
            matcher: Matcher::new(Config::DEFAULT),
            haystack: Vec::new(),
            needle: Vec::new(),
            indices: Vec::new(),
        }
    }

    fn find(&mut self, haystack: &str, needle: &str, casing: Casing) -> Option<Hit> {
        if needle.is_empty() {
            return None;
        }
        let ignore_case = match casing {
            Casing::Insensitive => true,
            Casing::Smart => !needle.chars().any(char::is_uppercase),
        };
        self.matcher.config.ignore_case = ignore_case;
        self.matcher.config.normalize = true;
        self.needle.clear();
        self.needle.extend(needle.chars().map(|ch| {
            let ch = normalize(ch);
            if ignore_case { to_lower_case(ch) } else { ch }
        }));
        self.haystack.clear();
        self.haystack.extend(haystack.chars());
        self.indices.clear();
        let haystack = if haystack.is_ascii() {
            Utf32Str::Ascii(haystack.as_bytes())
        } else {
            Utf32Str::Unicode(&self.haystack)
        };
        let ascii: Vec<u8>;
        let needle = if self.needle.iter().all(char::is_ascii) {
            ascii = self.needle.iter().map(|ch| *ch as u8).collect();
            Utf32Str::Ascii(&ascii)
        } else {
            Utf32Str::Unicode(&self.needle)
        };
        let score = self
            .matcher
            .fuzzy_indices(haystack, needle, &mut self.indices)?;
        Some(self.hit(score))
    }

    fn hit(&mut self, score: u16) -> Hit {
        let mut positions: Vec<usize> = self.indices.iter().map(|index| *index as usize).collect();
        positions.sort_unstable();
        positions.dedup();
        Hit {
            score: i64::from(score),
            start: positions.first().copied().unwrap_or(0),
            positions,
        }
    }
}

fn char_len(text: &str) -> usize {
    text.chars().count()
}

fn char_slice(text: &str, start: usize) -> &str {
    match text.char_indices().nth(start) {
        Some((byte, _)) => &text[byte..],
        None => "",
    }
}

fn base_name(path: &str) -> &str {
    path.rsplit_once('/').map_or(path, |(_, name)| name)
}

fn comparable(query: &str, value: &str) -> (String, String) {
    if query != query.to_lowercase() {
        (query.to_owned(), value.to_owned())
    } else {
        (query.to_owned(), value.to_lowercase())
    }
}

fn starts_with_query_case(value: &str, query: &str) -> bool {
    let (query, value) = comparable(query, value);
    value.starts_with(&query)
}

fn is_subsequence(query: &str, value: &str) -> bool {
    let mut needle = query.chars().peekable();
    for ch in value.chars() {
        if needle.peek() == Some(&ch) {
            needle.next();
        }
        if needle.peek().is_none() {
            return true;
        }
    }
    needle.peek().is_none()
}

fn path_relevance_bonus(path: &str, query: &str) -> i64 {
    let (query, value) = comparable(query, path);
    let base = base_name(&value);
    let mut bonus = 0;
    if base.contains(&query) {
        bonus += BASE_NAME_CONTAINS;
    }
    if is_subsequence(&query, base) {
        bonus += BASE_NAME_SUBSEQUENCE;
    }
    if value.contains(&query) {
        bonus += PATH_CONTAINS;
    }
    let hits = if query.is_empty() {
        0
    } else {
        value.matches(query.as_str()).count() as i64
    };
    bonus + hits * REPEATED_PATH_HIT
}

fn ranked(intent: PathIntent, score: i64) -> i64 {
    intent as i64 * RANK_UNIT + score
}

#[derive(Debug, Clone)]
struct PathMatch {
    index: usize,
    path: String,
    positions: Vec<usize>,
    score: i64,
    start: usize,
}

fn compare_path_matches(left: &PathMatch, right: &PathMatch) -> Ordering {
    right
        .score
        .cmp(&left.score)
        .then(left.start.cmp(&right.start))
        .then(char_len(&left.path).cmp(&char_len(&right.path)))
        .then_with(|| left.path.cmp(&right.path))
}

#[derive(Debug, Clone)]
struct Segment {
    text: String,
    start: usize,
    is_file_name: bool,
}

fn split_segments(path: &str) -> Vec<Segment> {
    let names: Vec<&str> = path.split('/').collect();
    let count = names.len();
    let mut start = 0;
    names
        .into_iter()
        .enumerate()
        .map(|(index, text)| {
            let segment = Segment {
                text: text.to_owned(),
                start,
                is_file_name: index + 1 == count,
            };
            start += char_len(text) + 1;
            segment
        })
        .collect()
}

#[derive(Debug, Clone)]
struct SegmentMatch {
    index: usize,
    positions: Vec<usize>,
    score: i64,
    start: usize,
}

fn segment_match(
    fzf: &mut Fzf,
    query: &str,
    segments: &[Segment],
    from: usize,
    skip_file_name: bool,
) -> Option<SegmentMatch> {
    let mut best: Option<(Hit, usize)> = None;
    for (index, segment) in segments.iter().enumerate() {
        if index < from || (skip_file_name && segment.is_file_name) {
            continue;
        }
        let Some(hit) = fzf.find(&segment.text, query, Casing::Smart) else {
            continue;
        };
        let better = match &best {
            None => true,
            Some((current, current_index)) => {
                hit.score > current.score
                    || (hit.score == current.score
                        && (hit.start < current.start
                            || (hit.start == current.start
                                && char_len(&segment.text)
                                    < char_len(&segments[*current_index].text))))
            }
        };
        if better {
            best = Some((hit, index));
        }
    }
    let (hit, index) = best?;
    let segment = &segments[index];
    Some(SegmentMatch {
        index,
        positions: hit
            .positions
            .iter()
            .map(|position| segment.start + position)
            .collect(),
        score: hit.score,
        start: segment.start + hit.start,
    })
}

fn directory_segment_prefix_match(index: usize, path: &str, query: &str) -> Option<PathMatch> {
    let query_len = char_len(query);
    if query_len < IMPLICIT_DIRECTORY_QUERY_MIN_LENGTH {
        return None;
    }
    let mut best: Option<(i64, usize)> = None;
    for segment in split_segments(path) {
        if segment.is_file_name {
            continue;
        }
        let (needle, value) = comparable(query, &segment.text);
        let mut score = if value == needle {
            DIRECTORY_SEGMENT_EXACT
        } else if value.starts_with(&needle) {
            DIRECTORY_SEGMENT_PREFIX - char_len(&segment.text).saturating_sub(query_len) as i64
        } else {
            continue;
        };
        if segment.start == 0 {
            score += ROOT_DIRECTORY_SEGMENT;
        }
        let better = match best {
            None => true,
            Some((best_score, best_start)) => {
                score > best_score || (score == best_score && segment.start < best_start)
            }
        };
        if better {
            best = Some((score, segment.start));
        }
    }
    let (score, start) = best?;
    Some(PathMatch {
        index,
        path: path.to_owned(),
        positions: (start..start + query_len).collect(),
        score: ranked(PathIntent::DirectorySegmentPrefix, score),
        start,
    })
}

fn split_query(query: &str) -> (&str, &str) {
    match query.rfind('/') {
        Some(index) => (&query[..=index], &query[index + 1..]),
        None => ("", query),
    }
}

fn exact_directory_prefix_match(
    fzf: &mut Fzf,
    index: usize,
    path: &str,
    query: &str,
) -> Option<PathMatch> {
    let (prefix, leaf) = split_query(query);
    if prefix.is_empty() || !starts_with_query_case(path, prefix) {
        return None;
    }
    let prefix_len = char_len(prefix);
    let prefix_positions: Vec<usize> = (0..prefix_len).collect();
    if leaf.is_empty() {
        return Some(PathMatch {
            index,
            path: path.to_owned(),
            positions: prefix_positions,
            score: ranked(PathIntent::ExactDirectoryPrefix, 0),
            start: 0,
        });
    }
    let tail = char_slice(path, prefix_len);
    let hit = fzf.find(tail, leaf, Casing::Smart)?;
    let mut positions = prefix_positions;
    positions.extend(hit.positions.iter().map(|position| prefix_len + position));
    positions.sort_unstable();
    positions.dedup();
    Some(PathMatch {
        index,
        path: path.to_owned(),
        positions,
        score: ranked(
            PathIntent::ExactDirectoryPrefix,
            hit.score + path_relevance_bonus(tail, leaf),
        ),
        start: 0,
    })
}

fn structured_path_match(
    fzf: &mut Fzf,
    index: usize,
    path: &str,
    query: &str,
) -> Option<PathMatch> {
    let query_segments: Vec<&str> = query.split('/').filter(|part| !part.is_empty()).collect();
    if query_segments.is_empty() || query_segments.len() > STRUCTURED_QUERY_SEGMENT_MAX_COUNT {
        return None;
    }
    let trailing_slash = query.ends_with('/');
    let segments = split_segments(path);
    let mut positions = Vec::new();
    let mut score = 0;
    let mut first_start: Option<usize> = None;
    let mut next = 0;
    for (query_index, part) in query_segments.iter().enumerate() {
        let found = segment_match(
            fzf,
            part,
            &segments,
            next,
            trailing_slash && query_index + 1 == query_segments.len(),
        )?;
        positions.extend(found.positions.iter().copied());
        first_start = Some(first_start.map_or(found.start, |start| start.min(found.start)));
        score += found.score;
        if found.index == next {
            score += CONSECUTIVE_SEGMENT;
        }
        next = found.index + 1;
    }
    let leaf_query = query_segments[query_segments.len() - 1];
    if let Some(leaf) = segments.last()
        && let Some(found) = segment_match(fzf, leaf_query, std::slice::from_ref(leaf), 0, false)
    {
        score += LEAF_SEGMENT + found.score
            - char_len(&leaf.text).saturating_sub(char_len(leaf_query)) as i64;
    }
    positions.sort_unstable();
    positions.dedup();
    Some(PathMatch {
        index,
        path: path.to_owned(),
        positions,
        score: ranked(PathIntent::StructuredPath, score),
        start: first_start.unwrap_or(0),
    })
}

fn plain_matches(fzf: &mut Fzf, paths: &[(usize, String)], query: &str) -> Vec<PathMatch> {
    let mut matches: Vec<PathMatch> = paths
        .iter()
        .filter_map(|(index, path)| {
            let hit = fzf.find(path, query, Casing::Smart)?;
            Some(PathMatch {
                index: *index,
                path: path.clone(),
                score: ranked(PathIntent::PlainFzf, hit.score) + path_relevance_bonus(path, query),
                start: hit.start,
                positions: hit.positions,
            })
        })
        .collect();
    matches.sort_by(compare_path_matches);
    matches
}

fn merge_path_matches(matches: Vec<PathMatch>) -> Vec<PathMatch> {
    let mut best: HashMap<String, PathMatch> = HashMap::new();
    for candidate in matches {
        match best.get(&candidate.path) {
            Some(existing) if compare_path_matches(&candidate, existing) != Ordering::Less => {}
            _ => {
                best.insert(candidate.path.clone(), candidate);
            }
        }
    }
    let mut merged: Vec<PathMatch> = best.into_values().collect();
    merged.sort_by(compare_path_matches);
    merged
}

fn rank_paths(paths: &[(usize, String)], query: &str) -> Vec<PathMatch> {
    if !query.is_empty() && query.split('/').all(str::is_empty) {
        return Vec::new();
    }
    let mut fzf = Fzf::new();
    if query.contains('/') {
        let exact: Vec<PathMatch> = paths
            .iter()
            .filter_map(|(index, path)| exact_directory_prefix_match(&mut fzf, *index, path, query))
            .collect();
        if !exact.is_empty() && query.ends_with('/') {
            return merge_path_matches(exact);
        }
        let structured: Vec<PathMatch> = paths
            .iter()
            .filter_map(|(index, path)| structured_path_match(&mut fzf, *index, path, query))
            .collect();
        if !exact.is_empty() {
            return merge_path_matches(exact.into_iter().chain(structured).collect());
        }
        let mut all = plain_matches(&mut fzf, paths, query);
        all.extend(structured);
        return merge_path_matches(all);
    }
    let mut all = plain_matches(&mut fzf, paths, query);
    all.extend(
        paths
            .iter()
            .filter_map(|(index, path)| directory_segment_prefix_match(*index, path, query)),
    );
    merge_path_matches(all)
}

fn first_items(count: usize, limit: usize) -> Vec<FuzzyMatch> {
    (0..count.min(limit))
        .map(|index| FuzzyMatch {
            index,
            score: 0,
            positions: Vec::new(),
        })
        .collect()
}

pub fn match_paths<T>(
    items: &[T],
    query: &str,
    path: impl Fn(&T) -> &str,
    limit: usize,
) -> Vec<FuzzyMatch> {
    if limit == 0 {
        return Vec::new();
    }
    if query.is_empty() {
        return first_items(items.len(), limit);
    }
    let query = query.replace('\\', "/");
    if char_len(&query) > QUERY_MAX_LENGTH {
        return Vec::new();
    }
    let paths: Vec<(usize, String)> = items
        .iter()
        .enumerate()
        .map(|(index, item)| (index, path(item).replace('\\', "/")))
        .collect();
    rank_paths(&paths, &query)
        .into_iter()
        .take(limit)
        .map(|found| FuzzyMatch {
            index: found.index,
            score: found.score,
            positions: found.positions,
        })
        .collect()
}

fn text_relevance_bonus(text: &str, query: &str) -> i64 {
    let value = text.to_lowercase();
    let query = query.to_lowercase();
    if value == query {
        TEXT_EXACT
    } else if value.starts_with(&query) {
        TEXT_PREFIX
    } else if value.contains(&query) {
        TEXT_CONTAINS
    } else if is_subsequence(&query, &value) {
        TEXT_SUBSEQUENCE
    } else {
        0
    }
}

#[derive(Debug, Clone)]
struct TextMatch {
    index: usize,
    text_index: usize,
    text: String,
    positions: Vec<usize>,
    score: i64,
    start: usize,
}

fn compare_text_matches(left: &TextMatch, right: &TextMatch) -> Ordering {
    right
        .score
        .cmp(&left.score)
        .then(left.start.cmp(&right.start))
        .then(left.text_index.cmp(&right.text_index))
        .then(char_len(&left.text).cmp(&char_len(&right.text)))
        .then_with(|| left.text.cmp(&right.text))
        .then(left.index.cmp(&right.index))
}

pub fn match_text<T>(
    items: &[T],
    query: &str,
    text: impl Fn(&T) -> &str,
    aliases: impl Fn(&T) -> Vec<&str>,
    limit: usize,
) -> Vec<FuzzyMatch> {
    if limit == 0 {
        return Vec::new();
    }
    let query = query.trim();
    if query.is_empty() {
        return first_items(items.len(), limit);
    }
    if char_len(query) > QUERY_MAX_LENGTH {
        return Vec::new();
    }
    let mut fzf = Fzf::new();
    let mut best: Vec<Option<TextMatch>> = vec![None; items.len()];
    for (index, item) in items.iter().enumerate() {
        let candidates = std::iter::once((0, text(item), false)).chain(
            aliases(item)
                .into_iter()
                .enumerate()
                .map(|(alias_index, alias)| (alias_index + 1, alias, true)),
        );
        for (text_index, candidate, is_alias) in candidates {
            if candidate.is_empty() {
                continue;
            }
            let Some(hit) = fzf.find(candidate, query, Casing::Insensitive) else {
                continue;
            };
            let found = TextMatch {
                index,
                text_index,
                text: candidate.to_owned(),
                score: hit.score
                    + text_relevance_bonus(candidate, query)
                    + if is_alias { 0 } else { PRIMARY_TEXT_SCORE },
                start: hit.start,
                positions: hit.positions,
            };
            let slot = &mut best[index];
            let replace = match slot {
                Some(existing) => compare_text_matches(&found, existing) == Ordering::Less,
                None => true,
            };
            if replace {
                *slot = Some(found);
            }
        }
    }
    let mut matches: Vec<TextMatch> = best.into_iter().flatten().collect();
    matches.sort_by(compare_text_matches);
    matches
        .into_iter()
        .take(limit)
        .map(|found| FuzzyMatch {
            index: found.index,
            score: found.score,
            positions: if found.text_index == 0 {
                found.positions
            } else {
                Vec::new()
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(items: &[&str], query: &str, limit: usize) -> Vec<String> {
        match_paths(items, query, |path| path, limit)
            .into_iter()
            .map(|found| items[found.index].to_owned())
            .collect()
    }

    fn texts(items: &[&str], query: &str) -> Vec<String> {
        match_text(items, query, |item| item, |_| Vec::new(), items.len())
            .into_iter()
            .map(|found| items[found.index].to_owned())
            .collect()
    }

    #[test]
    fn empty_query_limit_and_overlong_query() {
        assert_eq!(
            paths(&["src/a.ts", "src/b.ts", "src/c.ts"], "", 2),
            ["src/a.ts", "src/b.ts"]
        );
        assert!(paths(&["src/a.ts"], "a", 0).is_empty());
        assert!(paths(&["src/a.ts"], &"a".repeat(QUERY_MAX_LENGTH + 1), 10).is_empty());
        assert!(paths(&["src/a.ts", "plans/a.md"], "///", 10).is_empty());
        assert!(texts(&["Alpha"], &"a".repeat(QUERY_MAX_LENGTH + 1)).is_empty());
    }

    #[test]
    fn ranks_filename_and_boundary_matches_first() {
        let items = [
            "fixtures/generated/prompt-log.ts",
            "apps/app/src/components/promptbox/PromptBox.tsx",
            "apps/app/src/components/promptbox/PromptMentionMenu.tsx",
            "apps/app/src/hooks/usePromptDraftStorage.ts",
        ];
        let ranked = paths(&items, "prompt", 3);
        assert!(ranked[..2].contains(&items[1].to_owned()), "{ranked:?}");
        assert!(ranked[..2].contains(&items[2].to_owned()), "{ranked:?}");
        let subsequence = match_paths(&items[1..3], "pmtbx", |path| path, 5);
        assert_eq!(subsequence[0].index, 0);
        assert!(!subsequence[0].positions.is_empty());
    }

    #[test]
    fn backslash_paths_match_slash_queries() {
        assert_eq!(
            paths(
                &[
                    "apps\\app\\src\\components\\promptbox\\PromptBox.tsx",
                    "apps\\app\\src\\views\\PromptBox.tsx",
                ],
                "comp/prompt",
                5,
            ),
            ["apps\\app\\src\\components\\promptbox\\PromptBox.tsx"]
        );
    }

    #[test]
    fn exact_directory_prefix_scopes_results() {
        assert_eq!(
            paths(
                &[
                    "apps/cli/src/commands/guide.ts",
                    "plans/c.md",
                    "plans/b.md",
                    "plans/a.md",
                    "apps/cli/src/commands/status.ts",
                ],
                "plans/",
                8,
            ),
            ["plans/a.md", "plans/b.md", "plans/c.md"]
        );
        let exact = match_paths(&["plans/code-quality-follow-ups.md"], "plans/", |p| p, 1);
        let unrelated = match_paths(&["plans-local/local.pak"], "plans/", |p| p, 1);
        assert!(exact[0].score > unrelated.first().map_or(0, |found| found.score));
        assert_eq!(
            paths(
                &[
                    "plans/code-quality-follow-ups.md",
                    "plans/timeline-bundle-unification.md",
                    "packages/generated/unification.ts",
                ],
                "plans/uni",
                8,
            ),
            ["plans/timeline-bundle-unification.md"]
        );
    }

    #[test]
    fn directory_names_rank_before_slash_is_typed() {
        let items = [
            "packages/db/test/query-plans.test.ts",
            "packages/workflow-runtime/src/playback.ts",
            "packages/agent-runtime/src/codex/generated/codex-app-server/schema/v2/TurnPlanStep.ts",
            "apps/server/src/services/threads/thread-execution-plan.ts",
            "plans/askuserquestion-option-preview.md",
            "plans/background-command-support.md",
            "plans/environment-branch-display.md",
        ];
        for query in ["pl", "pla", "plan", "plans"] {
            let ranked = paths(&items, query, 3);
            assert!(
                ranked.iter().all(|path| path.starts_with("plans/")),
                "{query}: {ranked:?}"
            );
        }
    }

    #[test]
    fn structured_queries_require_each_segment() {
        assert!(
            !paths(
                &[
                    "packages/generated/unification.ts",
                    "plans/timeline-bundle-unification.md",
                    "docs/plans-unification.md",
                ],
                "plaxs/uni",
                8,
            )
            .contains(&"plans/timeline-bundle-unification.md".to_owned())
        );
        assert!(paths(&["plans-foo-bar/file.ts"], "plnas/file", 10).is_empty());
        assert_eq!(
            paths(
                &[
                    "apps/app/src/components/pickers/EnvironmentPicker.tsx",
                    "apps/app/src/views/PromptBox.tsx",
                    "apps/app/src/components/promptbox/PromptBox.tsx",
                ],
                "app/comp/pmtbx",
                8,
            )[0],
            "apps/app/src/components/promptbox/PromptBox.tsx"
        );
        assert_eq!(
            paths(
                &["src/foo/util-helper.ts", "src/other/readme.md"],
                "src//util",
                10
            ),
            ["src/foo/util-helper.ts"]
        );
    }

    #[test]
    fn structured_prefix_prefers_short_leaves() {
        let items = [
            "apps/cli/src/commands/machine-enrollment.test.ts",
            "apps/cli/src/commands/machine-enrollment.ts",
            "apps/cli/src/commands/machine-environment.ts",
            "apps/cli/src/commands/machine.ts",
        ];
        let ranked = match_paths(&items, "commands/machine", |path| path, 8);
        assert_eq!(items[ranked[0].index], "apps/cli/src/commands/machine.ts");
        assert!(ranked[0].score > ranked[1].score);
        assert_eq!(
            items[ranked[3].index],
            "apps/cli/src/commands/machine-enrollment.test.ts"
        );
        let tie = match_paths(
            &["src/a/extra/file.ts", "src/a/file.ts"],
            "src/file",
            |path| path,
            8,
        );
        assert_eq!(tie[0].index, 1);
        assert_eq!(tie[0].score, tie[1].score);
    }

    #[test]
    fn exact_prefix_and_structured_matches_merge() {
        assert_eq!(
            paths(
                &[
                    "src/test/test/file.ts",
                    "src/test/file.ts",
                    "test/src/file.ts"
                ],
                "test/file",
                10,
            ),
            [
                "test/src/file.ts",
                "src/test/file.ts",
                "src/test/test/file.ts"
            ]
        );
        let found = match_paths(&["src/foo/util-helper.ts"], "src/util", |path| path, 10);
        assert_eq!(found[0].positions, [0, 1, 2, 3, 8, 9, 10, 11]);
    }

    #[test]
    fn unicode_paths_match() {
        assert_eq!(
            paths(&["docs/café.md", "docs/cafe-notes.md"], "café", 10)[0],
            "docs/café.md"
        );
    }

    #[test]
    fn text_whitespace_and_case() {
        let items = ["New thread", "New  thread", "Next thread"];
        assert_eq!(texts(&items, "  new  thread  "), ["New  thread"]);
        assert_eq!(
            match_text(&items, "  ", |item| item, |_| Vec::new(), 2).len(),
            2
        );
        assert_eq!(texts(&["atlas-web", "Beacon API"], "AW"), ["atlas-web"]);
        assert_eq!(
            texts(&["atlas-web", "Beacon API"], "beacon api"),
            ["Beacon API"]
        );
    }

    #[test]
    fn text_aliases_rank_below_primary_matches() {
        let threads = [
            ("thr_research", "Research notes"),
            ("thr_prompt", "Prompt mention improvements"),
            ("thr_release", "Release checklist"),
        ];
        let found = match_text(
            &threads,
            "pmi",
            |thread| thread.1,
            |thread| vec![thread.0],
            3,
        );
        assert_eq!(found.len(), 1);
        assert_eq!(threads[found[0].index].0, "thr_prompt");
        assert!(!found[0].positions.is_empty());
        let by_id = match_text(
            &[
                ("thr_alpha", "Design review"),
                ("thr_beta", "Implementation plan"),
            ],
            "beta",
            |thread| thread.1,
            |thread| vec![thread.0],
            3,
        );
        assert_eq!(by_id.len(), 1);
        assert_eq!(by_id[0].index, 1);
        let models = [
            (
                "openrouter/nvidia/nemotron-3-ultra-550b-a55b",
                "NVIDIA: Nemotron 3 Ultra",
            ),
            ("openai/gpt-5.5", "GPT-5.5"),
        ];
        let ranked = match_text(&models, "55", |model| model.1, |model| vec![model.0], 2);
        assert_eq!(
            ranked.iter().map(|found| found.index).collect::<Vec<_>>(),
            [1, 0]
        );
        let ties = match_text(
            &[("thr_b", "Shared title"), ("thr_a", "Shared title")],
            "shared",
            |thread| thread.1,
            |thread| vec![thread.0],
            3,
        );
        assert_eq!(
            ties.iter().map(|found| found.index).collect::<Vec<_>>(),
            [0, 1]
        );
    }
}
