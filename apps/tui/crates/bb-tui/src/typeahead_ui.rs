use std::collections::HashMap;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};
use ratatui_textarea::DataCursor;
use unicode_width::UnicodeWidthStr;

use crate::app::{AppState, Mode};
use crate::attach::file_name;
use crate::compose::PickerItem;
use crate::composer::Composer;
use crate::composer_ui::render_item_list;
use crate::keymap::Command;
use crate::mentions::{CommandSuggestion, MentionSuggestion, SuggestionGroup};
use crate::prompt::{AttachmentKind, DraftAttachment};
use crate::typeahead::{ComposerTarget, MenuState};
use crate::ui::truncate;
use crate::wrap::truncate_spans;

const POPUP_MAX_ROWS: usize = 12;
const POPUP_MAX_WIDTH: u16 = 80;

fn muted() -> Style {
    Style::new().fg(Color::DarkGray)
}

pub fn chip_style() -> Style {
    Style::new()
        .fg(Color::Cyan)
        .bg(Color::Indexed(236))
        .add_modifier(Modifier::BOLD)
}

pub fn render_editor(composer: &Composer, focused: bool, frame: &mut Frame, area: Rect) {
    let unfocused;
    let textarea = if focused {
        composer.textarea()
    } else {
        let mut copy = composer.textarea().clone();
        copy.set_cursor_style(Style::default());
        unfocused = copy;
        &unfocused
    };
    frame.render_widget(textarea, area);
    let ranges = composer.mention_ranges();
    if ranges.is_empty() {
        return;
    }
    let lines = textarea.lines();
    let mut line_starts = Vec::with_capacity(lines.len());
    let mut offset = 0;
    for line in lines {
        line_starts.push(offset);
        offset += line.chars().count() + 1;
    }
    let (scroll_row, scroll_col) = textarea.scroll_offset();
    let style = chip_style();
    let buffer = frame.buffer_mut();
    for y in 0..area.height {
        for x in 0..area.width {
            let DataCursor(row, col) =
                textarea.screen_to_data(usize::from(y + scroll_row), usize::from(x + scroll_col));
            let Some(line) = lines.get(row) else {
                continue;
            };
            let Some(ch) = line.chars().nth(col) else {
                continue;
            };
            let char_offset = line_starts[row] + col;
            if !ranges
                .iter()
                .any(|range| range.start <= char_offset && char_offset < range.end)
            {
                continue;
            }
            if let Some(cell) = buffer.cell_mut((area.x + x, area.y + y))
                && cell.symbol().starts_with(ch)
            {
                cell.set_style(style);
            }
        }
    }
}

fn attachment_glyph(attachment: &DraftAttachment) -> &'static str {
    match attachment.kind {
        AttachmentKind::LocalImage => "▣",
        AttachmentKind::LocalFile => "▤",
    }
}

pub fn attachment_line(
    state: &AppState,
    composer: &Composer,
    width: usize,
) -> Option<Line<'static>> {
    let attachments = &composer.draft().attachments;
    if attachments.is_empty() && composer.uploads.is_empty() {
        return None;
    }
    let mut spans = Vec::new();
    for attachment in attachments {
        spans.push(Span::styled(
            format!(" {} {} ", attachment_glyph(attachment), attachment.name),
            chip_style(),
        ));
        spans.push(Span::raw(" "));
    }
    for upload in &composer.uploads {
        spans.push(Span::styled(
            format!(" … {} ", file_name(std::path::Path::new(upload))),
            muted().add_modifier(Modifier::ITALIC),
        ));
        spans.push(Span::raw(" "));
    }
    if !attachments.is_empty() {
        spans.push(Span::styled(
            format!("{} remove", state.keymap.label_for(Command::Attachments)),
            muted(),
        ));
    }
    Some(Line::from(truncate_spans(spans, width)))
}

fn mention_row(
    item: &MentionSuggestion,
    labels: &HashMap<String, String>,
) -> (String, Option<String>) {
    match item {
        MentionSuggestion::Path {
            path, entry_kind, ..
        } => {
            let suffix = if *entry_kind == bb_wire::prompt::PathEntryKind::Directory
                && !path.ends_with('/')
            {
                "/"
            } else {
                ""
            };
            (format!("{path}{suffix}"), None)
        }
        MentionSuggestion::Thread {
            title,
            thread_id,
            project_id,
            project_name,
            relation,
            ..
        } => {
            let mut detail: Vec<String> = Vec::new();
            detail.extend(labels.get(project_id).or(project_name.as_ref()).cloned());
            detail.extend(relation.map(|relation| relation.label().to_owned()));
            (
                title.clone().unwrap_or_else(|| thread_id.clone()),
                (!detail.is_empty()).then(|| detail.join(" · ")),
            )
        }
        MentionSuggestion::Project {
            project_id, name, ..
        } => (labels.get(project_id).unwrap_or(name).clone(), None),
        MentionSuggestion::Section { name, .. } => (name.clone(), None),
        MentionSuggestion::Plugin {
            title, subtitle, ..
        } => (title.clone(), subtitle.clone()),
    }
}

fn command_row(trigger: char, item: &CommandSuggestion) -> (String, Option<String>) {
    let mut detail: Vec<String> = Vec::new();
    detail.extend(item.argument_hint.clone().filter(|hint| !hint.is_empty()));
    detail.extend(item.description.clone().filter(|text| !text.is_empty()));
    (
        format!("{trigger}{}", item.name),
        (!detail.is_empty()).then(|| detail.join(" — ")),
    )
}

struct Row {
    label: String,
    detail: Option<String>,
    index: Option<usize>,
}

fn grouped_rows<T>(
    groups: &[SuggestionGroup<T>],
    row: impl Fn(&T) -> (String, Option<String>),
) -> Vec<Row> {
    let mut rows = Vec::new();
    for group in groups {
        rows.push(Row {
            label: group.label.clone(),
            detail: None,
            index: None,
        });
        for (offset, item) in group.items.iter().enumerate() {
            let (label, detail) = row(item);
            rows.push(Row {
                label,
                detail,
                index: Some(group.start + offset),
            });
        }
    }
    rows
}

pub fn render_typeahead(
    state: &AppState,
    target: &ComposerTarget,
    frame: &mut Frame,
    editor: Rect,
) {
    let Some(typeahead) = state
        .typeahead
        .as_ref()
        .filter(|typeahead| typeahead.target == *target && typeahead.menu.is_visible())
    else {
        return;
    };
    let trigger = typeahead.trigger.ch;
    let rows: Vec<Row> = match &typeahead.menu {
        MenuState::Hidden => return,
        MenuState::Hint(text) | MenuState::Loading(text) => vec![Row {
            label: (*text).to_owned(),
            detail: None,
            index: None,
        }],
        MenuState::Error(error) => vec![Row {
            label: format!("Search failed: {error}"),
            detail: None,
            index: None,
        }],
        MenuState::Mentions(ordered) if ordered.is_empty() => vec![Row {
            label: "No matches".into(),
            detail: None,
            index: None,
        }],
        MenuState::Mentions(ordered) => {
            let labels = state
                .snapshot
                .sidebar
                .data
                .as_deref()
                .map(crate::sidebar::project_labels)
                .unwrap_or_default();
            grouped_rows(&ordered.groups, |item| mention_row(item, &labels))
        }
        MenuState::Commands(ordered) => {
            grouped_rows(&ordered.groups, |item| command_row(trigger, item))
        }
    };
    let screen = frame.area();
    let width = POPUP_MAX_WIDTH.min(editor.width.max(30)).min(screen.width);
    let inner_width = usize::from(width.saturating_sub(2));
    let selected_row = rows
        .iter()
        .position(|row| row.index == Some(typeahead.selected))
        .unwrap_or(0);
    let visible = POPUP_MAX_ROWS.min(rows.len()).max(1);
    let start = (selected_row + 1).saturating_sub(visible);
    let height = (visible as u16 + 2).min(screen.height);
    let space_above = editor.y.saturating_sub(screen.y);
    let y = if space_above >= height {
        editor.y - height
    } else {
        (editor.y + editor.height).min(screen.y + screen.height.saturating_sub(height))
    };
    let x = editor.x.min(screen.x + screen.width.saturating_sub(width));
    let rect = Rect {
        x,
        y,
        width,
        height,
    };
    let lines: Vec<Line> = rows
        .iter()
        .skip(start)
        .take(visible)
        .map(|row| match row.index {
            None if matches!(
                typeahead.menu,
                MenuState::Mentions(_) | MenuState::Commands(_)
            ) && !typeahead.menu.is_empty() =>
            {
                Line::styled(
                    truncate(&row.label, inner_width),
                    muted().add_modifier(Modifier::BOLD),
                )
            }
            None => Line::styled(truncate(&row.label, inner_width), muted()),
            Some(index) => {
                let mut label_style = Style::new();
                if index == typeahead.selected {
                    label_style = label_style.bg(Color::Blue).fg(Color::White);
                }
                let mut spans = vec![Span::styled(format!(" {}", row.label), label_style)];
                if let Some(detail) = &row.detail {
                    spans.push(Span::styled(format!("  {detail}"), muted()));
                }
                Line::from(truncate_spans(spans, inner_width))
            }
        })
        .collect();
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(Style::new().fg(Color::Cyan))
                .title(format!(" {trigger}{} ", typeahead.trigger.query)),
        ),
        rect,
    );
}

pub fn render_attach_modes(state: &AppState, frame: &mut Frame, area: Rect) {
    match &state.mode {
        Mode::AttachPrompt { input, .. } => {
            let width = area.width.saturating_sub(4).clamp(20, 90).min(area.width);
            let rect = Rect {
                x: area.x + (area.width - width) / 2,
                y: area.y + area.height.saturating_sub(5) / 2,
                width,
                height: 3.min(area.height),
            };
            let inner_width = usize::from(width.saturating_sub(4));
            let text = input.text();
            let shown_width = text.width();
            let (shown, cursor) = if shown_width > inner_width {
                let skip = shown_width - inner_width;
                let mut dropped = 0;
                let tail: String = text
                    .chars()
                    .skip_while(|ch| {
                        let keep = dropped >= skip;
                        if !keep {
                            dropped += ch.to_string().width();
                        }
                        !keep
                    })
                    .collect();
                let cursor = input.cursor_width().saturating_sub(dropped);
                (tail, cursor)
            } else {
                (text.to_owned(), input.cursor_width())
            };
            frame.render_widget(Clear, rect);
            frame.render_widget(
                Paragraph::new(Line::from(vec![Span::raw(" "), Span::raw(shown)])).block(
                    Block::bordered()
                        .border_type(BorderType::Rounded)
                        .border_style(Style::new().fg(Color::Cyan))
                        .title(" Attach file · Tab completes · Enter uploads "),
                ),
                rect,
            );
            frame.set_cursor_position((rect.x + 2 + cursor as u16, rect.y + 1));
        }
        Mode::Attachments { target, selected } => {
            let Some(composer) = state.composer_for(target) else {
                return;
            };
            let items: Vec<PickerItem> = composer
                .draft()
                .attachments
                .iter()
                .map(|attachment| PickerItem {
                    label: format!("{} {}", attachment_glyph(attachment), attachment.name),
                    detail: Some(attachment_detail(attachment)),
                    disabled: false,
                    current: false,
                })
                .collect();
            let refs: Vec<&PickerItem> = items.iter().collect();
            render_item_list("Attachments", &refs, *selected, None, frame, area);
        }
        _ => {}
    }
}

fn attachment_detail(attachment: &DraftAttachment) -> String {
    let mut parts = Vec::new();
    if let Some(size) = attachment.size_bytes {
        parts.push(format_size(size));
    }
    if let Some(mime) = &attachment.mime_type {
        parts.push(mime.clone());
    }
    if parts.is_empty() {
        parts.push(attachment.path.clone());
    }
    parts.join(" · ")
}

pub fn format_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}
