use bb_wire::timeline::PresentationBadgeTone;
use unicode_segmentation::UnicodeSegmentation;

use crate::domain::display_width;
use crate::format::{duration_ms_to_compact_string, format_diff_count};
use crate::title::{
    SegmentAccent, StatusDecorationStatus, TimelineTitle, TitleDecoration, TitleTone,
    format_timeline_decoration_text,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SpanRole {
    #[default]
    Text,
    Emphasis,
    Muted,
    Subtle,
    File,
    Link,
    Decoration,
    Error,
    Warning,
    Badge,
    BadgeDestructive,
    DiffAdded,
    DiffRemoved,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StyledSpan {
    pub text: String,
    pub role: SpanRole,
    pub shimmer: bool,
    pub truncate: bool,
}

impl StyledSpan {
    pub fn new(text: impl Into<String>, role: SpanRole) -> Self {
        Self {
            text: text.into(),
            role,
            ..Self::default()
        }
    }
}

pub fn spans_text(spans: &[StyledSpan]) -> String {
    spans.iter().map(|span| span.text.as_str()).collect()
}

pub fn spans_width(spans: &[StyledSpan]) -> usize {
    spans.iter().map(|span| display_width(&span.text)).sum()
}

fn segment_role(
    em: bool,
    accent: Option<SegmentAccent>,
    linked: bool,
    tone: TitleTone,
) -> SpanRole {
    if linked {
        return SpanRole::Link;
    }
    match accent {
        Some(SegmentAccent::File) => SpanRole::File,
        Some(SegmentAccent::Muted) => SpanRole::Muted,
        Some(SegmentAccent::Subtle) => SpanRole::Subtle,
        None if tone == TitleTone::Summary => SpanRole::Subtle,
        None if em => SpanRole::Emphasis,
        None => SpanRole::Text,
    }
}

fn decoration_spans(decoration: &TitleDecoration, now_ms: i64) -> Vec<StyledSpan> {
    match decoration {
        TitleDecoration::Duration {
            started_at,
            completed_at: None,
            em,
        } => {
            let elapsed = now_ms - started_at;
            if elapsed < 1_000 {
                return Vec::new();
            }
            vec![StyledSpan::new(
                duration_ms_to_compact_string(elapsed),
                if *em {
                    SpanRole::Emphasis
                } else {
                    SpanRole::Decoration
                },
            )]
        }
        TitleDecoration::Status { status, .. } => vec![StyledSpan::new(
            format_timeline_decoration_text(decoration),
            match status {
                StatusDecorationStatus::Error | StatusDecorationStatus::Denied => SpanRole::Error,
                StatusDecorationStatus::Interrupted => SpanRole::Warning,
            },
        )],
        TitleDecoration::SummaryStatus { error_count, .. } => vec![StyledSpan::new(
            format_timeline_decoration_text(decoration),
            if *error_count > 0 {
                SpanRole::Error
            } else {
                SpanRole::Warning
            },
        )],
        TitleDecoration::DiffStats { added, removed } => {
            let mut spans = Vec::new();
            if *added > 0 {
                spans.push(StyledSpan::new(
                    format!("+{}", format_diff_count(*added)),
                    SpanRole::DiffAdded,
                ));
            }
            if *removed > 0 {
                if !spans.is_empty() {
                    spans.push(StyledSpan::new(" ", SpanRole::Text));
                }
                spans.push(StyledSpan::new(
                    format!("-{}", format_diff_count(*removed)),
                    SpanRole::DiffRemoved,
                ));
            }
            spans
        }
        TitleDecoration::Badge { tone, .. } => vec![StyledSpan::new(
            format_timeline_decoration_text(decoration),
            if *tone == PresentationBadgeTone::Destructive {
                SpanRole::BadgeDestructive
            } else {
                SpanRole::Badge
            },
        )],
        TitleDecoration::Duration { .. } => vec![StyledSpan::new(
            unparenthesized(format_timeline_decoration_text(decoration)),
            SpanRole::Decoration,
        )],
    }
}

fn unparenthesized(text: String) -> String {
    match text
        .strip_prefix('(')
        .and_then(|rest| rest.strip_suffix(')'))
    {
        Some(inner) => inner.to_owned(),
        None => text,
    }
}

pub fn title_spans(title: &TimelineTitle, now_ms: i64) -> Vec<StyledSpan> {
    let mut spans: Vec<StyledSpan> = Vec::new();
    let push_with_gap = |spans: &mut Vec<StyledSpan>, group: Vec<StyledSpan>| {
        if group.is_empty() || group.iter().all(|span| span.text.is_empty()) {
            return;
        }
        if !spans.is_empty() {
            spans.push(StyledSpan::new(" ", SpanRole::Text));
        }
        spans.extend(group);
    };
    for segment in &title.segments {
        if segment.text.is_empty() {
            continue;
        }
        push_with_gap(
            &mut spans,
            vec![StyledSpan {
                text: segment.text.clone(),
                role: segment_role(
                    segment.em,
                    segment.accent,
                    segment.link.is_some(),
                    title.tone,
                ),
                shimmer: segment.shimmer,
                truncate: segment.truncate,
            }],
        );
    }
    for decoration in &title.decorations {
        push_with_gap(&mut spans, decoration_spans(decoration, now_ms));
    }
    spans
}

fn truncate_to_width(text: &str, max_width: usize) -> String {
    let mut width = 0;
    let mut out = String::new();
    for grapheme in text.graphemes(true) {
        let next = width + display_width(grapheme);
        if next > max_width {
            break;
        }
        width = next;
        out.push_str(grapheme);
    }
    out
}

pub fn fit_spans_to_width(spans: &[StyledSpan], max_width: usize) -> Vec<StyledSpan> {
    let mut out = spans.to_vec();
    let mut overflow = spans_width(&out).saturating_sub(max_width);
    for span in out.iter_mut().rev() {
        if overflow == 0 {
            break;
        }
        if !span.truncate {
            continue;
        }
        let width = display_width(&span.text);
        if width <= 1 {
            continue;
        }
        let target = width.saturating_sub(overflow + 1).max(1);
        let truncated = format!("{}…", truncate_to_width(&span.text, target));
        overflow = overflow.saturating_sub(width - display_width(&truncated));
        span.text = truncated;
    }
    if overflow > 0 {
        let mut remaining = max_width;
        let mut clipped = Vec::new();
        for mut span in out {
            if remaining == 0 {
                break;
            }
            let width = display_width(&span.text);
            if width > remaining {
                span.text = truncate_to_width(&span.text, remaining);
                remaining = 0;
            } else {
                remaining -= width;
            }
            clipped.push(span);
        }
        return clipped;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::title::TitleSegment;

    fn title(segments: Vec<TitleSegment>, decorations: Vec<TitleDecoration>) -> TimelineTitle {
        TimelineTitle {
            segments,
            decorations,
            tone: TitleTone::Default,
            action: None,
            plain: String::new(),
        }
    }

    fn seg(text: &str, em: bool, truncate: bool) -> TitleSegment {
        TitleSegment {
            text: text.into(),
            em,
            truncate,
            ..TitleSegment::default()
        }
    }

    #[test]
    fn renders_live_durations_and_roles() {
        let title = title(
            vec![seg("Running", false, false), seg("cargo test", true, true)],
            vec![
                TitleDecoration::Duration {
                    started_at: 1_000,
                    completed_at: None,
                    em: false,
                },
                TitleDecoration::DiffStats {
                    added: 3,
                    removed: 0,
                },
            ],
        );
        let spans = title_spans(&title, 66_000);
        assert_eq!(spans_text(&spans), "Running cargo test 1m 5s +3");
        assert_eq!(spans[2].role, SpanRole::Emphasis);
        assert_eq!(spans[6].role, SpanRole::DiffAdded);
        assert_eq!(
            spans_text(&title_spans(&title, 1_500)),
            "Running cargo test +3"
        );
    }

    #[test]
    fn truncates_only_truncatable_spans() {
        let spans = title_spans(
            &title(
                vec![
                    seg("Ran", false, false),
                    seg("a-very-long-command-line", true, true),
                ],
                vec![TitleDecoration::Status {
                    status: StatusDecorationStatus::Error,
                    duration_ms: None,
                    emphasis: false,
                }],
            ),
            0,
        );
        let fitted = fit_spans_to_width(&spans, 20);
        assert_eq!(spans_text(&fitted), "Ran a-very-… (error)");
        assert_eq!(spans_width(&fitted), 20);
        assert_eq!(fitted[4].role, SpanRole::Error);
        assert_eq!(spans_text(&fit_spans_to_width(&spans, 6)), "Ran a…");
    }
}
