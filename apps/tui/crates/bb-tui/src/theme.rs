use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};

pub const LIGHT_SYNTAX_THEME: &str = "InspiredGitHub";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Appearance {
    #[default]
    Auto,
    Dark,
    Light,
}

impl Appearance {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "dark" => Some(Self::Dark),
            "light" => Some(Self::Light),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Theme {
    pub no_color: bool,
    pub light: bool,
}

pub fn no_color_requested(env: &impl Fn(&str) -> Option<String>) -> bool {
    env("NO_COLOR").is_some_and(|value| !value.is_empty())
}

pub fn detect_light(env: &impl Fn(&str) -> Option<String>) -> Option<bool> {
    let value = env("COLORFGBG")?;
    let background = value.rsplit(';').next()?.trim().parse::<u8>().ok()?;
    Some(matches!(background, 7 | 9..=15))
}

pub fn resolve(appearance: Appearance, env: impl Fn(&str) -> Option<String>) -> Theme {
    let light = match appearance {
        Appearance::Light => true,
        Appearance::Dark => false,
        Appearance::Auto => detect_light(&env).unwrap_or(false),
    };
    Theme {
        no_color: no_color_requested(&env),
        light,
    }
}

fn light_fg(color: Color) -> Color {
    match color {
        Color::White => Color::Black,
        Color::Gray => Color::DarkGray,
        Color::Yellow => Color::Rgb(150, 105, 0),
        Color::Cyan => Color::Rgb(0, 120, 140),
        Color::LightYellow => Color::Rgb(150, 105, 0),
        other => other,
    }
}

fn light_bg(color: Color) -> Color {
    match color {
        Color::DarkGray => Color::Gray,
        Color::Rgb(18, 48, 24) => Color::Rgb(218, 251, 225),
        Color::Rgb(56, 20, 22) => Color::Rgb(255, 235, 233),
        other => other,
    }
}

fn emphasizes(color: Color) -> bool {
    matches!(
        color,
        Color::Blue
            | Color::Red
            | Color::Green
            | Color::Yellow
            | Color::Cyan
            | Color::Magenta
            | Color::DarkGray
            | Color::Gray
            | Color::White
            | Color::LightBlue
            | Color::LightRed
    )
}

pub fn apply(theme: Theme, buffer: &mut Buffer) {
    if theme.no_color {
        for cell in &mut buffer.content {
            if emphasizes(cell.bg) {
                cell.modifier.insert(Modifier::REVERSED);
            }
            cell.fg = Color::Reset;
            cell.bg = Color::Reset;
        }
        return;
    }
    if theme.light {
        for cell in &mut buffer.content {
            if matches!(
                cell.bg,
                Color::Reset | Color::DarkGray | Color::Rgb(18, 48, 24) | Color::Rgb(56, 20, 22)
            ) {
                cell.fg = light_fg(cell.fg);
            }
            cell.bg = light_bg(cell.bg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;
    use ratatui::style::Style;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        }
    }

    #[test]
    fn no_color_needs_a_non_empty_value() {
        assert!(resolve(Appearance::Auto, env(&[("NO_COLOR", "1")])).no_color);
        assert!(!resolve(Appearance::Auto, env(&[("NO_COLOR", "")])).no_color);
        assert!(!resolve(Appearance::Auto, env(&[])).no_color);
    }

    #[test]
    fn auto_reads_the_colorfgbg_background() {
        assert!(resolve(Appearance::Auto, env(&[("COLORFGBG", "0;15")])).light);
        assert!(resolve(Appearance::Auto, env(&[("COLORFGBG", "0;default;7")])).light);
        assert!(!resolve(Appearance::Auto, env(&[("COLORFGBG", "15;0")])).light);
        assert!(!resolve(Appearance::Auto, env(&[("COLORFGBG", "junk")])).light);
        assert!(!resolve(Appearance::Dark, env(&[("COLORFGBG", "0;15")])).light);
        assert!(resolve(Appearance::Light, env(&[])).light);
    }

    #[test]
    fn no_color_strips_colors_and_reverses_highlights() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 3, 1));
        buffer.set_string(0, 0, "a", Style::new().fg(Color::Red));
        buffer.set_string(1, 0, "b", Style::new().bg(Color::Blue).fg(Color::White));
        buffer.set_string(2, 0, "c", Style::new().bg(Color::Rgb(18, 48, 24)));
        apply(
            Theme {
                no_color: true,
                light: false,
            },
            &mut buffer,
        );
        for cell in &buffer.content {
            assert_eq!((cell.fg, cell.bg), (Color::Reset, Color::Reset));
        }
        assert!(!buffer.content[0].modifier.contains(Modifier::REVERSED));
        assert!(buffer.content[1].modifier.contains(Modifier::REVERSED));
        assert!(!buffer.content[2].modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn light_swaps_colors_that_vanish_on_a_light_background() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 2, 1));
        buffer.set_string(0, 0, "a", Style::new().fg(Color::White));
        buffer.set_string(1, 0, "b", Style::new().bg(Color::DarkGray));
        apply(
            Theme {
                no_color: false,
                light: true,
            },
            &mut buffer,
        );
        assert_eq!(buffer.content[0].fg, Color::Black);
        assert_eq!(buffer.content[1].bg, Color::Gray);
    }
}
