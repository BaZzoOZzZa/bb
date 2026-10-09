use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

fn basic_color(index: u16, bright: bool) -> Color {
    match (index, bright) {
        (0, false) => Color::Black,
        (1, false) => Color::Red,
        (2, false) => Color::Green,
        (3, false) => Color::Yellow,
        (4, false) => Color::Blue,
        (5, false) => Color::Magenta,
        (6, false) => Color::Cyan,
        (7, false) => Color::Gray,
        (0, true) => Color::DarkGray,
        (1, true) => Color::LightRed,
        (2, true) => Color::LightGreen,
        (3, true) => Color::LightYellow,
        (4, true) => Color::LightBlue,
        (5, true) => Color::LightMagenta,
        (6, true) => Color::LightCyan,
        _ => Color::White,
    }
}

fn extended_color(params: &mut std::slice::Iter<'_, u16>) -> Option<Color> {
    match params.next()? {
        5 => params.next().map(|index| Color::Indexed(*index as u8)),
        2 => {
            let r = *params.next()? as u8;
            let g = *params.next()? as u8;
            let b = *params.next()? as u8;
            Some(Color::Rgb(r, g, b))
        }
        _ => None,
    }
}

fn apply_sgr(style: Style, base: Style, params: &[u16]) -> Style {
    if params.is_empty() {
        return base;
    }
    let mut style = style;
    let mut iter = params.iter();
    while let Some(code) = iter.next() {
        style = match code {
            0 => base,
            1 => style.add_modifier(Modifier::BOLD),
            2 => style.add_modifier(Modifier::DIM),
            3 => style.add_modifier(Modifier::ITALIC),
            4 => style.add_modifier(Modifier::UNDERLINED),
            7 => style.add_modifier(Modifier::REVERSED),
            9 => style.add_modifier(Modifier::CROSSED_OUT),
            22 => style.remove_modifier(Modifier::BOLD | Modifier::DIM),
            23 => style.remove_modifier(Modifier::ITALIC),
            24 => style.remove_modifier(Modifier::UNDERLINED),
            27 => style.remove_modifier(Modifier::REVERSED),
            29 => style.remove_modifier(Modifier::CROSSED_OUT),
            30..=37 => style.fg(basic_color(code - 30, false)),
            90..=97 => style.fg(basic_color(code - 90, true)),
            40..=47 => style.bg(basic_color(code - 40, false)),
            100..=107 => style.bg(basic_color(code - 100, true)),
            38 => match extended_color(&mut iter) {
                Some(color) => style.fg(color),
                None => style,
            },
            48 => match extended_color(&mut iter) {
                Some(color) => style.bg(color),
                None => style,
            },
            39 => match base.fg {
                Some(color) => style.fg(color),
                None => style.fg(Color::Reset),
            },
            49 => match base.bg {
                Some(color) => style.bg(color),
                None => style.bg(Color::Reset),
            },
            _ => style,
        };
    }
    style
}

fn visible_line(line: &str) -> &str {
    let line = line.strip_suffix('\r').unwrap_or(line);
    match line.rfind('\r') {
        Some(index) => &line[index + 1..],
        None => line,
    }
}

pub fn ansi_lines(text: &str, base: Style) -> Vec<Vec<Span<'static>>> {
    let mut style = base;
    let mut out = Vec::new();
    for raw in text.split('\n') {
        let line = visible_line(raw);
        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut current = String::new();
        let mut chars = line.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch != '\u{1b}' {
                if ch == '\t' {
                    current.push_str(crate::wrap::TAB);
                } else if !ch.is_control() {
                    current.push(ch);
                }
                continue;
            }
            match chars.peek().copied() {
                Some('[') => {
                    chars.next();
                    let mut body = String::new();
                    let mut terminator = None;
                    for next in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&next) {
                            terminator = Some(next);
                            break;
                        }
                        body.push(next);
                    }
                    if terminator == Some('m') {
                        if !current.is_empty() {
                            spans.push(Span::styled(std::mem::take(&mut current), style));
                        }
                        let params: Vec<u16> = body
                            .split([';', ':'])
                            .map(|part| part.parse().unwrap_or(0))
                            .collect();
                        let params = if body.is_empty() { Vec::new() } else { params };
                        style = apply_sgr(style, base, &params);
                    }
                }
                Some(']') => {
                    chars.next();
                    while let Some(next) = chars.next() {
                        if next == '\u{7}' {
                            break;
                        }
                        if next == '\u{1b}' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                Some(_) => {
                    chars.next();
                }
                None => {}
            }
        }
        if !current.is_empty() {
            spans.push(Span::styled(current, style));
        }
        out.push(spans);
    }
    out
}

pub fn strip_ansi(text: &str) -> String {
    ansi_lines(text, Style::new())
        .into_iter()
        .map(|spans| {
            spans
                .into_iter()
                .map(|span| span.content)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sgr_colors_and_resets() {
        let base = Style::new().fg(Color::Gray);
        let lines = ansi_lines("ok \u{1b}[1;31merror\u{1b}[0m done\n\u{1b}[38;5;42mx", base);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0][0].content, "ok ");
        assert_eq!(lines[0][1].content, "error");
        assert_eq!(
            lines[0][1].style,
            base.fg(Color::Red).add_modifier(Modifier::BOLD)
        );
        assert_eq!(lines[0][2].style, base);
        assert_eq!(lines[1][0].style.fg, Some(Color::Indexed(42)));
    }

    #[test]
    fn drops_cursor_sequences_osc_and_carriage_returns() {
        let text = "progress 10%\rprogress 100%\r\n\u{1b}]0;title\u{7}\u{1b}[2Kdone";
        assert_eq!(strip_ansi(text), "progress 100%\ndone");
    }
}
