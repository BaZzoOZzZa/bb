use std::sync::OnceLock;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle, Theme, ThemeSet};
use syntect::parsing::{SyntaxReference, SyntaxSet};
use syntect::util::LinesWithEndings;

const MAX_HIGHLIGHT_BYTES: usize = 200_000;
const THEME: &str = "base16-ocean.dark";

struct Highlighter {
    syntaxes: SyntaxSet,
    theme: Theme,
}

fn highlighter() -> &'static Highlighter {
    static HIGHLIGHTER: OnceLock<Highlighter> = OnceLock::new();
    HIGHLIGHTER.get_or_init(|| {
        let mut themes = ThemeSet::load_defaults();
        let theme = themes
            .themes
            .remove(THEME)
            .or_else(|| themes.themes.into_values().next())
            .unwrap_or_default();
        Highlighter {
            syntaxes: SyntaxSet::load_defaults_newlines(),
            theme,
        }
    })
}

fn alias(lang: &str) -> &str {
    match lang {
        "console" | "shellscript" | "shell" | "sh" | "zsh" | "fish" | "bash" => "bash",
        "h" => "c",
        "hpp" | "hh" | "hxx" => "cpp",
        "less" => "css",
        "ts" | "tsx" | "typescript" | "mts" | "cts" => "ts",
        "jsx" | "mjs" | "cjs" => "js",
        "yml" => "yaml",
        "rs" => "rust",
        "py" => "python",
        "rb" => "ruby",
        "golang" => "go",
        "jsonc" | "json5" => "json",
        "toml" => "toml",
        other => other,
    }
}

fn find_syntax<'a>(syntaxes: &'a SyntaxSet, lang: &str) -> Option<&'a SyntaxReference> {
    let lang = lang
        .split([' ', ',', '{'])
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if lang.is_empty() {
        return None;
    }
    let lang = alias(&lang);
    syntaxes
        .find_syntax_by_token(lang)
        .or_else(|| syntaxes.find_syntax_by_extension(lang))
        .or_else(|| match lang {
            "ts" => syntaxes.find_syntax_by_token("js"),
            "toml" => syntaxes.find_syntax_by_token("ini"),
            _ => None,
        })
}

fn convert(style: syntect::highlighting::Style, base: Style) -> Style {
    let fg = style.foreground;
    let mut out = base.fg(Color::Rgb(fg.r, fg.g, fg.b));
    if style.font_style.contains(FontStyle::BOLD) {
        out = out.add_modifier(Modifier::BOLD);
    }
    if style.font_style.contains(FontStyle::ITALIC) {
        out = out.add_modifier(Modifier::ITALIC);
    }
    out
}

fn plain_lines(code: &str, base: Style) -> Vec<Vec<Span<'static>>> {
    code.split('\n')
        .map(|line| vec![Span::styled(line.replace('\t', crate::wrap::TAB), base)])
        .collect()
}

pub fn supports_language(lang: &str) -> bool {
    find_syntax(&highlighter().syntaxes, lang).is_some()
}

pub fn highlight_code(code: &str, lang: Option<&str>, base: Style) -> Vec<Vec<Span<'static>>> {
    let code = code.strip_suffix('\n').unwrap_or(code);
    let Some(lang) = lang.filter(|lang| !lang.trim().is_empty()) else {
        return plain_lines(code, base);
    };
    if code.len() > MAX_HIGHLIGHT_BYTES {
        return plain_lines(code, base);
    }
    let highlighter = highlighter();
    let Some(syntax) = find_syntax(&highlighter.syntaxes, lang) else {
        return plain_lines(code, base);
    };
    let mut lines = HighlightLines::new(syntax, &highlighter.theme);
    let mut out = Vec::new();
    for line in LinesWithEndings::from(code) {
        let Ok(ranges) = lines.highlight_line(line, &highlighter.syntaxes) else {
            return plain_lines(code, base);
        };
        let spans = ranges
            .into_iter()
            .filter_map(|(style, text)| {
                let text = text
                    .trim_end_matches(['\n', '\r'])
                    .replace('\t', crate::wrap::TAB);
                (!text.is_empty()).then(|| Span::styled(text, convert(style, base)))
            })
            .collect();
        out.push(spans);
    }
    if out.is_empty() {
        out.push(Vec::new());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlights_known_languages_and_falls_back() {
        let lines = highlight_code(
            "fn main() {\n    let x = 1;\n}\n",
            Some("rust"),
            Style::new(),
        );
        assert_eq!(lines.len(), 3);
        let text: String = lines[0].iter().map(|span| span.content.as_ref()).collect();
        assert_eq!(text, "fn main() {");
        assert!(lines[0].len() > 1);
        assert!(
            lines[0]
                .iter()
                .all(|span| matches!(span.style.fg, Some(Color::Rgb(..))))
        );
        let plain = highlight_code("a\tb", Some("no-such-lang"), Style::new());
        assert_eq!(plain, vec![vec![Span::raw("a    b")]]);
        assert!(supports_language("tsx"));
        assert!(supports_language("sh"));
    }
}
