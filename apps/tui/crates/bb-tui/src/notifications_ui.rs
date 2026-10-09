use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::app::{AppState, Mode};
use crate::notifications::{Notice, NoticeKind};
use crate::time::compact_relative_time;
use crate::ui::truncate;

const MAX_WIDTH: u16 = 96;
const MAX_HEIGHT: u16 = 24;

fn kind_style(kind: NoticeKind) -> Style {
    match kind {
        NoticeKind::NeedsInput => Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        NoticeKind::Error => Style::new().fg(Color::Red),
        NoticeKind::TurnFinished => Style::new().fg(Color::Green),
        NoticeKind::Plugin => Style::new().fg(Color::Cyan),
    }
}

fn notice_line(notice: &Notice, now_ms: i64, width: usize, selected: bool) -> Line<'static> {
    let time = format!("{:>4} ", compact_relative_time(notice.at_ms, now_ms));
    let marker = if selected { "› " } else { "  " };
    let glyph = format!("{} ", notice.kind.glyph());
    let used = marker.width() + glyph.width() + time.width();
    let available = width.saturating_sub(used);
    let title = truncate(&notice.title, available.min(available / 2 + 8).max(1));
    let rest = available.saturating_sub(title.width() + 2);
    let body = if notice.body.is_empty() || rest < 4 {
        String::new()
    } else {
        format!("  {}", truncate(&notice.body.replace('\n', " "), rest))
    };
    let base = if selected {
        Style::new().add_modifier(Modifier::REVERSED)
    } else {
        Style::new()
    };
    let title_style = if notice.unread {
        base.add_modifier(Modifier::BOLD)
    } else {
        base
    };
    Line::from(vec![
        Span::styled(marker, base),
        Span::styled(glyph, kind_style(notice.kind)),
        Span::styled(time, Style::new().fg(Color::DarkGray)),
        Span::styled(title, title_style),
        Span::styled(body, Style::new().fg(Color::Gray)),
    ])
}

pub fn render_notifications(state: &AppState, frame: &mut Frame, area: Rect) {
    let Mode::Notifications { selected } = state.mode else {
        return;
    };
    let width = area.width.min(MAX_WIDTH);
    let height = area.height.min(MAX_HEIGHT);
    if width < 20 || height < 4 {
        return;
    }
    let rect = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };
    let inner_width = usize::from(width.saturating_sub(2));
    let visible = usize::from(height.saturating_sub(3)).max(1);
    let total = state.notifications.len();
    let offset = selected.saturating_sub(visible.saturating_sub(1));
    let mut lines: Vec<Line> = if total == 0 {
        vec![Line::styled(
            "  No notifications yet",
            Style::new().fg(Color::DarkGray),
        )]
    } else {
        state
            .notifications
            .newest_first()
            .enumerate()
            .skip(offset)
            .take(visible)
            .map(|(index, notice)| {
                notice_line(notice, state.now_ms, inner_width, index == selected)
            })
            .collect()
    };
    while lines.len() < visible {
        lines.push(Line::default());
    }
    lines.push(Line::styled(
        truncate(
            " Enter open · j/k move · d delete · C clear all · Esc close",
            inner_width,
        ),
        Style::new().fg(Color::DarkGray),
    ));
    let title = if total == 0 {
        " Notifications ".to_owned()
    } else {
        format!(" Notifications ({total}) ")
    };
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .title(title),
        ),
        rect,
    );
}
