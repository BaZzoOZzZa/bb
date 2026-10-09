use bb_wire::timeline::{CommandWorkRow, TimelineRowStatus};

use crate::domain::is_raw_thread_id;

const SHELL_WRAPPER_NAMES: [&str; 3] = ["sh", "bash", "zsh"];
const SHELL_SEGMENT_BREAK_TOKENS: [&str; 5] = ["&&", "||", "|", ";", "\n"];
const DOUBLE_QUOTE_ESCAPABLE: [char; 5] = ['$', '`', '"', '\\', '\n'];
const THREAD_TELL_VALUE_FLAGS: [&str; 6] = [
    "--message-file",
    "--model",
    "--service-tier",
    "--reasoning-level",
    "--permission-mode",
    "--mode",
];
const THREAD_TELL_BOOLEAN_FLAGS: [&str; 2] = ["--json", "--plan"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellToken {
    pub value: String,
    pub quoted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadTellCommand {
    pub thread_id: String,
    pub message: String,
}

fn unwrap_quoted_shell_arg(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() < 2 {
        return value.to_owned();
    }
    let quote = chars[0];
    if (quote != '\'' && quote != '"') || chars[chars.len() - 1] != quote {
        return value.to_owned();
    }
    let inner = &chars[1..chars.len() - 1];
    if quote == '\'' {
        return inner.iter().collect();
    }
    let mut result = String::new();
    let mut index = 0;
    while index < inner.len() {
        let ch = inner[index];
        if ch == '\\'
            && index + 1 < inner.len()
            && DOUBLE_QUOTE_ESCAPABLE.contains(&inner[index + 1])
        {
            result.push(inner[index + 1]);
            index += 2;
            continue;
        }
        result.push(ch);
        index += 1;
    }
    result
}

fn is_known_shell_wrapper(value: &str) -> bool {
    let name = value.rsplit('/').next().unwrap_or(value);
    SHELL_WRAPPER_NAMES.contains(&name)
}

pub fn extract_shell_command_from_string(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let Some(program_end) = trimmed.find(char::is_whitespace) else {
        return Some(trimmed.to_owned());
    };
    let program = &trimmed[..program_end];
    let rest = trimmed[program_end..].trim_start();
    let flag_rest = rest
        .strip_prefix("-lc")
        .filter(|tail| tail.starts_with(char::is_whitespace))
        .or_else(|| {
            rest.strip_prefix("-c")
                .filter(|tail| tail.starts_with(char::is_whitespace))
        });
    let Some(command_arg) = flag_rest.map(str::trim_start).filter(|arg| !arg.is_empty()) else {
        return Some(trimmed.to_owned());
    };
    if !is_known_shell_wrapper(program) {
        return Some(trimmed.to_owned());
    }
    Some(unwrap_quoted_shell_arg(command_arg.trim()))
}

struct SegmentCollector<'f> {
    segment: Vec<ShellToken>,
    visit: &'f mut dyn FnMut(Vec<ShellToken>) -> bool,
}

impl SegmentCollector<'_> {
    fn append(&mut self, token: ShellToken) -> bool {
        if !token.quoted && SHELL_SEGMENT_BREAK_TOKENS.contains(&token.value.as_str()) {
            let completed = std::mem::take(&mut self.segment);
            return completed.is_empty() || (self.visit)(completed);
        }
        self.segment.push(token);
        true
    }

    fn op(&mut self, value: &str) -> bool {
        self.append(ShellToken {
            value: value.to_owned(),
            quoted: false,
        })
    }
}

#[derive(Default)]
struct CurrentWord {
    text: String,
    has_quoted: bool,
    has_unquoted: bool,
}

impl CurrentWord {
    fn flush(&mut self, collector: &mut SegmentCollector<'_>) -> bool {
        let fully_quoted = self.has_quoted && !self.has_unquoted;
        if self.text.is_empty() && !fully_quoted {
            self.has_quoted = false;
            self.has_unquoted = false;
            return true;
        }
        let token = ShellToken {
            value: std::mem::take(&mut self.text),
            quoted: fully_quoted,
        };
        self.has_quoted = false;
        self.has_unquoted = false;
        collector.append(token)
    }
}

pub fn visit_shell_command_segments(command: &str, visit: &mut dyn FnMut(Vec<ShellToken>) -> bool) {
    let mut collector = SegmentCollector {
        segment: Vec::new(),
        visit,
    };
    let chars: Vec<char> = command.chars().collect();
    let mut current = CurrentWord::default();
    let mut quote: Option<char> = None;
    let mut escaping = false;
    let mut index = 0;
    while index < chars.len() {
        let ch = chars[index];
        if escaping {
            current.text.push(ch);
            if quote.is_some() {
                current.has_quoted = true;
            } else {
                current.has_unquoted = true;
            }
            escaping = false;
            index += 1;
            continue;
        }
        if ch == '\\' {
            if quote == Some('\'') {
                current.text.push(ch);
                current.has_quoted = true;
                index += 1;
                continue;
            }
            if quote == Some('"') {
                let next = chars.get(index + 1);
                if next.is_none_or(|next| !DOUBLE_QUOTE_ESCAPABLE.contains(next)) {
                    current.text.push(ch);
                    current.has_quoted = true;
                    index += 1;
                    continue;
                }
            }
            escaping = true;
            index += 1;
            continue;
        }
        if let Some(open) = quote {
            if ch == open {
                quote = None;
                if current.text.is_empty() {
                    current.has_quoted = true;
                }
            } else {
                current.text.push(ch);
                current.has_quoted = true;
            }
            index += 1;
            continue;
        }
        if ch == '\'' || ch == '"' {
            quote = Some(ch);
            index += 1;
            continue;
        }
        if ch == '\n' {
            if !current.flush(&mut collector) || !collector.op("\n") {
                return;
            }
            index += 1;
            continue;
        }
        if ch.is_whitespace() {
            if !current.flush(&mut collector) {
                return;
            }
            index += 1;
            continue;
        }
        if ch == '|' || ch == '&' || ch == ';' {
            if ch == '&' && chars.get(index + 1) == Some(&'>') {
                if !current.flush(&mut collector) {
                    return;
                }
                if chars.get(index + 2) == Some(&'>') {
                    if !collector.op("&>>") {
                        return;
                    }
                    index += 3;
                } else {
                    if !collector.op("&>") {
                        return;
                    }
                    index += 2;
                }
                continue;
            }
            if !current.flush(&mut collector) {
                return;
            }
            let next = chars.get(index + 1).copied();
            if (ch == '|' && next == Some('|')) || (ch == '&' && next == Some('&')) {
                if !collector.op(&format!("{ch}{ch}")) {
                    return;
                }
                index += 2;
                continue;
            }
            if !collector.op(&ch.to_string()) {
                return;
            }
            index += 1;
            continue;
        }
        if ch == '<' || ch == '>' {
            let mut prefix = String::new();
            let current_is_unquoted = current.has_unquoted && !current.has_quoted;
            if current_is_unquoted
                && (current.text == "&"
                    || (!current.text.is_empty()
                        && current.text.chars().all(|c| c.is_ascii_digit())))
            {
                prefix = std::mem::take(&mut current.text);
                current.has_unquoted = false;
            } else if (!current.text.is_empty() || current.has_quoted)
                && !current.flush(&mut collector)
            {
                return;
            }
            let next1 = chars.get(index + 1).copied();
            let next2 = chars.get(index + 2).copied();
            let (op, consumed) = if ch == '>' {
                match next1 {
                    Some('>') => (">>", 2),
                    Some('|') => (">|", 2),
                    Some('(') => (">(", 2),
                    Some('&') => (">&", 2),
                    _ => (">", 1),
                }
            } else {
                match (next1, next2) {
                    (Some('<'), Some('<')) => ("<<<", 3),
                    (Some('<'), Some('-')) => ("<<-", 3),
                    (Some('<'), _) => ("<<", 2),
                    (Some('('), _) => ("<(", 2),
                    (Some('>'), _) => ("<>", 2),
                    _ => ("<", 1),
                }
            };
            if !collector.op(&format!("{prefix}{op}")) {
                return;
            }
            index += consumed;
            continue;
        }
        current.text.push(ch);
        current.has_unquoted = true;
        index += 1;
    }
    if escaping {
        current.text.push('\\');
        current.has_unquoted = true;
    }
    if !current.flush(&mut collector) {
        return;
    }
    if !collector.segment.is_empty() {
        let segment = std::mem::take(&mut collector.segment);
        (collector.visit)(segment);
    }
}

pub(crate) struct RedirectScan {
    pub consumed_extra: usize,
}

fn split_redirect_prefix(value: &str) -> (&str, &str) {
    if let Some(rest) = value.strip_prefix('&')
        && (rest.starts_with('<') || rest.starts_with('>'))
    {
        return ("&", rest);
    }
    let digits = value.chars().take_while(char::is_ascii_digit).count();
    value.split_at(digits)
}

pub(crate) fn scan_redirect_at(tokens: &[ShellToken], index: usize) -> Option<RedirectScan> {
    let token = tokens.get(index)?;
    if token.quoted {
        return None;
    }
    let value = token.value.as_str();
    let has_next = tokens.get(index + 1).is_some();
    let (_, op) = split_redirect_prefix(value);
    let scan = |consumed_extra| Some(RedirectScan { consumed_extra });
    match op {
        "<<" | "<<-" | "<<<" | "<>" | "<" | ">|" => return scan(1),
        _ => {}
    }
    if value == "<(" || value == ">(" {
        let mut extra = 0;
        for token in &tokens[index + 1..] {
            extra += 1;
            if token.value.ends_with(')') {
                break;
            }
        }
        return scan(extra);
    }
    if op == ">&" || op == ">>&" {
        return scan(usize::from(has_next));
    }
    if op == ">" || op == ">>" {
        return scan(usize::from(has_next));
    }
    None
}

fn is_env_assignment(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    for ch in chars {
        if ch == '=' {
            return true;
        }
        if !(ch.is_ascii_alphanumeric() || ch == '_') {
            return false;
        }
    }
    false
}

pub(crate) fn command_token_index(tokens: &[ShellToken]) -> usize {
    tokens
        .iter()
        .take_while(|token| !token.quoted && is_env_assignment(&token.value))
        .count()
}

pub(crate) fn base_executable_name(token: &str) -> &str {
    token.rsplit('/').next().unwrap_or(token)
}

fn is_word_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn has_thread_tell_hint(command: &str) -> bool {
    let mut search_from = 0;
    while let Some(offset) = command[search_from..].find("thread") {
        let start = search_from + offset;
        search_from = start + "thread".len();
        if command[..start]
            .chars()
            .next_back()
            .is_some_and(is_word_char)
        {
            continue;
        }
        let after = &command[search_from..];
        let trimmed = after.trim_start();
        if trimmed.len() == after.len() {
            continue;
        }
        for verb in ["tell", "message"] {
            if let Some(tail) = trimmed.strip_prefix(verb)
                && !tail.chars().next().is_some_and(is_word_char)
            {
                return true;
            }
        }
    }
    false
}

fn is_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
        && chars.all(is_word_char)
}

fn take_identifier(value: &str) -> Option<(&str, &str)> {
    let end = value
        .char_indices()
        .find(|(index, ch)| {
            if *index == 0 {
                !(ch.is_ascii_alphabetic() || *ch == '_')
            } else {
                !is_word_char(*ch)
            }
        })
        .map_or(value.len(), |(index, _)| index);
    if end == 0 {
        None
    } else {
        Some(value.split_at(end))
    }
}

fn is_bb_cli_token(value: &str) -> bool {
    let Some(rest) = value.strip_prefix('$') else {
        return false;
    };
    let rest = rest.strip_prefix('{').unwrap_or(rest);
    let Some(rest) = rest.strip_prefix("BB_CLI") else {
        return false;
    };
    let rest = match rest.strip_prefix(":-") {
        Some(default) => match default.find('}') {
            Some(index) => &default[index..],
            None => "",
        },
        None => rest,
    };
    rest.is_empty() || rest == "}"
}

fn shell_variable_name(value: &str) -> Option<&str> {
    let rest = value.strip_prefix('$')?;
    let rest = rest.strip_prefix('{').unwrap_or(rest);
    let name = rest.strip_suffix('}').unwrap_or(rest);
    is_identifier(name).then_some(name)
}

struct HeredocHeader<'a> {
    dash: bool,
    quote: &'a str,
    delimiter: &'a str,
    body_start: &'a str,
}

fn parse_heredoc_header(after_marker: &str) -> Option<HeredocHeader<'_>> {
    let (dash, rest) = match after_marker.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, after_marker),
    };
    let rest = rest.trim_start_matches([' ', '\t']);
    let (quote, rest) = match rest.chars().next() {
        Some(q @ ('\'' | '"')) => (&rest[..1], &rest[q.len_utf8()..]),
        _ => ("", rest),
    };
    let (delimiter, rest) = take_identifier(rest)?;
    let rest = rest.strip_prefix(quote)?;
    let rest = rest.trim_start_matches([' ', '\t']);
    let body_start = rest.strip_prefix('\n')?;
    Some(HeredocHeader {
        dash,
        quote,
        delimiter,
        body_start,
    })
}

fn heredoc_body(body: &str, dash: bool, quote: &str, terminator_tabs: &str) -> Option<String> {
    if !terminator_tabs.is_empty() && !dash {
        return None;
    }
    if quote.is_empty() && body.contains(['$', '`', '\\']) {
        return None;
    }
    if dash {
        Some(
            body.split('\n')
                .map(|line| line.trim_start_matches('\t'))
                .collect::<Vec<_>>()
                .join("\n"),
        )
    } else {
        Some(body.to_owned())
    }
}

fn split_terminator<'a>(line: &'a str, delimiter: &str) -> Option<&'a str> {
    let tabs_end = line.len() - line.trim_start_matches('\t').len();
    (&line[tabs_end..] == delimiter).then(|| &line[..tabs_end])
}

fn match_heredoc_assignment(line: &str) -> Option<(String, String, String)> {
    let (name, rest) = take_identifier(line)?;
    let rest = rest.strip_prefix("=$(cat <<")?;
    let header = parse_heredoc_header(rest)?;
    let body_region = header.body_start;
    let newline_positions: Vec<usize> = body_region
        .char_indices()
        .filter(|(_, ch)| *ch == '\n')
        .map(|(index, _)| index)
        .collect();
    for &position in &newline_positions {
        let body = &body_region[..position];
        let tail = &body_region[position + 1..];
        let mut lines = tail.splitn(4, '\n');
        let (Some(terminator), Some(closer), Some(rest_line)) =
            (lines.next(), lines.next(), lines.next())
        else {
            continue;
        };
        if lines.next().is_some() || rest_line.contains('\n') {
            continue;
        }
        let Some(tabs) = split_terminator(terminator, header.delimiter) else {
            continue;
        };
        if closer.trim_matches([' ', '\t']) != ")" {
            continue;
        }
        let value = heredoc_body(body, header.dash, header.quote, tabs)?;
        return Some((name.to_owned(), value, rest_line.to_owned()));
    }
    None
}

fn match_stdin_heredoc(line: &str) -> Option<Option<(String, String)>> {
    let first_line_end = line.find('\n').unwrap_or(line.len());
    let mut search_from = 0;
    while let Some(offset) = line[search_from..first_line_end].find("<<") {
        let marker = search_from + offset;
        search_from = marker + 1;
        let head = &line[..marker];
        let Some(header) = parse_heredoc_header(&line[marker + 2..]) else {
            continue;
        };
        let region = header.body_start;
        let Some(last_newline) = region.rfind('\n') else {
            continue;
        };
        let body = &region[..last_newline];
        let Some(tabs) = split_terminator(&region[last_newline + 1..], header.delimiter) else {
            continue;
        };
        return Some(
            heredoc_body(body, header.dash, header.quote, tabs)
                .map(|stdin| (head.to_owned(), stdin)),
        );
    }
    None
}

struct ThreadTellScript {
    line: String,
    stdin: Option<String>,
    variable: Option<(String, String)>,
}

fn split_thread_tell_script(script: &str) -> Option<ThreadTellScript> {
    let mut line = script.trim().to_owned();
    let mut variable = None;
    if let Some((name, value, rest)) = match_heredoc_assignment(&line) {
        variable = Some((name, value));
        line = rest.trim().to_owned();
    }
    let mut stdin = None;
    if let Some(piped) = match_stdin_heredoc(&line) {
        let (head, body) = piped?;
        stdin = Some(body);
        line = head.trim().to_owned();
    }
    Some(ThreadTellScript {
        line,
        stdin,
        variable,
    })
}

fn resolve_thread_tell_message(value: &str, variable: Option<&(String, String)>) -> Option<String> {
    if let Some(name) = shell_variable_name(value) {
        return variable
            .filter(|(variable_name, _)| variable_name == name)
            .map(|(_, value)| value.clone());
    }
    if value.contains(['$', '`']) {
        None
    } else {
        Some(value.to_owned())
    }
}

fn parse_thread_tell_segment(
    tokens: &[ShellToken],
    script: &ThreadTellScript,
) -> Option<ThreadTellCommand> {
    let tokens = &tokens[command_token_index(tokens)..];
    let [cli, group, verb, arg_tokens @ ..] = tokens else {
        return None;
    };
    if (base_executable_name(&cli.value) != "bb" && !is_bb_cli_token(&cli.value))
        || group.value != "thread"
        || (verb.value != "tell" && verb.value != "message")
    {
        return None;
    }
    let mut positionals: Vec<&str> = Vec::new();
    let mut message_file: Option<String> = None;
    let mut index = 0;
    while index < arg_tokens.len() {
        let token = &arg_tokens[index];
        if let Some(redirect) = scan_redirect_at(arg_tokens, index) {
            index += redirect.consumed_extra + 1;
            continue;
        }
        if !token.quoted && token.value.starts_with('-') && token.value != "-" {
            let (flag, inline_value) = match token.value.split_once('=') {
                Some((flag, value)) => (flag, Some(value)),
                None => (token.value.as_str(), None),
            };
            if THREAD_TELL_BOOLEAN_FLAGS.contains(&flag) && inline_value.is_none() {
                index += 1;
                continue;
            }
            if !THREAD_TELL_VALUE_FLAGS.contains(&flag) {
                return None;
            }
            let value = match inline_value {
                Some(value) => Some(value.to_owned()),
                None => {
                    index += 1;
                    arg_tokens.get(index).map(|token| token.value.clone())
                }
            };
            if flag == "--message-file" {
                message_file = value;
            }
            index += 1;
            continue;
        }
        positionals.push(&token.value);
        index += 1;
    }
    let (thread_id, message_arg, extra) = match positionals.as_slice() {
        [] => (None, None, false),
        [thread_id] => (Some(*thread_id), None, false),
        [thread_id, message, rest @ ..] => (Some(*thread_id), Some(*message), !rest.is_empty()),
    };
    let message = match (message_arg, message_file.as_deref()) {
        (Some(arg), None) => resolve_thread_tell_message(arg, script.variable.as_ref()),
        (None, Some("-")) => script.stdin.clone(),
        _ => None,
    };
    let thread_id = thread_id.filter(|id| is_raw_thread_id(id))?;
    let message = message.filter(|message| !message.trim().is_empty())?;
    if extra {
        return None;
    }
    Some(ThreadTellCommand {
        thread_id: thread_id.to_owned(),
        message,
    })
}

pub fn parse_thread_tell_command(command: &str) -> Option<ThreadTellCommand> {
    if !has_thread_tell_hint(command) {
        return None;
    }
    let unwrapped = extract_shell_command_from_string(command)?;
    let script = split_thread_tell_script(&unwrapped)?;
    let mut segments: Vec<Vec<ShellToken>> = Vec::new();
    visit_shell_command_segments(&script.line, &mut |segment| {
        segments.push(segment);
        segments.len() < 2
    });
    match segments.as_slice() {
        [segment] => parse_thread_tell_segment(segment, &script),
        _ => None,
    }
}

pub fn parse_sent_thread_message(row: &CommandWorkRow) -> Option<ThreadTellCommand> {
    if row.status != TimelineRowStatus::Completed || row.exit_code.is_some_and(|code| code != 0) {
        return None;
    }
    parse_thread_tell_command(&row.command)
}

#[cfg(test)]
mod tests {
    use super::*;

    const THREAD: &str = "thr_23456789ab";

    fn tell(command: &str) -> Option<ThreadTellCommand> {
        parse_thread_tell_command(command)
    }

    fn expected(message: &str) -> Option<ThreadTellCommand> {
        Some(ThreadTellCommand {
            thread_id: THREAD.into(),
            message: message.into(),
        })
    }

    fn tokens(command: &str) -> Vec<Vec<(String, bool)>> {
        let mut out = Vec::new();
        visit_shell_command_segments(command, &mut |segment| {
            out.push(
                segment
                    .into_iter()
                    .map(|token| (token.value, token.quoted))
                    .collect(),
            );
            true
        });
        out
    }

    #[test]
    fn tokenizes_quotes_operators_and_redirects() {
        assert_eq!(
            tokens("echo 'a b' \"c\\\"d\" && ls 2>/dev/null | wc"),
            vec![
                vec![
                    ("echo".to_owned(), false),
                    ("a b".to_owned(), true),
                    ("c\"d".to_owned(), true)
                ],
                vec![
                    ("ls".to_owned(), false),
                    ("2>".to_owned(), false),
                    ("/dev/null".to_owned(), false)
                ],
                vec![("wc".to_owned(), false)],
            ]
        );
        assert_eq!(
            tokens("cat <<EOF &>> log"),
            vec![vec![
                ("cat".to_owned(), false),
                ("<<".to_owned(), false),
                ("EOF".to_owned(), false),
                ("&>>".to_owned(), false),
                ("log".to_owned(), false),
            ]]
        );
        assert_eq!(
            tokens("echo ''"),
            vec![vec![("echo".to_owned(), false), (String::new(), true)]]
        );
    }

    #[test]
    fn unwraps_shell_wrappers() {
        assert_eq!(
            extract_shell_command_from_string("/bin/bash -lc 'bb thread tell x'").as_deref(),
            Some("bb thread tell x")
        );
        assert_eq!(
            extract_shell_command_from_string("zsh -c \"echo \\\"hi\\\"\"").as_deref(),
            Some("echo \"hi\"")
        );
        assert_eq!(
            extract_shell_command_from_string("python -c 'x'").as_deref(),
            Some("python -c 'x'")
        );
        assert_eq!(extract_shell_command_from_string("   "), None);
    }

    #[test]
    fn parses_direct_thread_tell_commands() {
        assert_eq!(
            tell(&format!("bb thread tell {THREAD} 'hello there'")),
            expected("hello there")
        );
        assert_eq!(
            tell(&format!(
                "bash -lc \"bb thread message {THREAD} hi --json\""
            )),
            expected("hi")
        );
        assert_eq!(
            tell(&format!(
                "FOO=1 \"$BB_CLI\" thread tell --model gpt {THREAD} hi"
            )),
            expected("hi")
        );
        assert_eq!(
            tell(&format!(
                "FOO=1 $BB_CLI thread tell --model gpt {THREAD} hi"
            )),
            expected("hi")
        );
        assert_eq!(
            tell(&format!(
                "${{BB_CLI:-bb}} thread tell {THREAD} hi 2>/dev/null"
            )),
            expected("hi")
        );
    }

    #[test]
    fn rejects_unsafe_or_ambiguous_thread_tell_commands() {
        assert_eq!(tell(&format!("bb thread tell {THREAD} \"$MSG\"")), None);
        assert_eq!(tell(&format!("bb thread tell {THREAD} hi extra")), None);
        assert_eq!(tell(&format!("bb thread tell {THREAD} hi --unknown")), None);
        assert_eq!(tell("bb thread tell thr_bad hi"), None);
        assert_eq!(
            tell(&format!("bb thread tell {THREAD} hi && echo done")),
            None
        );
        assert_eq!(tell(&format!("bb thread tell {THREAD} '   '")), None);
        assert_eq!(tell(&format!("bb threads tell {THREAD} hi")), None);
    }

    #[test]
    fn reads_heredoc_messages() {
        assert_eq!(
            tell(&format!(
                "bb thread tell {THREAD} --message-file - <<'EOF'\nline one\nline $two\nEOF"
            )),
            expected("line one\nline $two")
        );
        assert_eq!(
            tell(&format!(
                "bb thread tell {THREAD} --message-file - <<EOF\nline $two\nEOF"
            )),
            None
        );
        assert_eq!(
            tell(&format!(
                "MSG=$(cat <<'EOF'\nhello\nworld\nEOF\n)\nbb thread tell {THREAD} \"$MSG\""
            )),
            expected("hello\nworld")
        );
        assert_eq!(
            tell(&format!(
                "bb thread tell {THREAD} --message-file - <<-EOF\n\tindented\n\tEOF"
            )),
            expected("indented")
        );
    }

    #[test]
    fn requires_successful_completed_commands() {
        let mut row = CommandWorkRow {
            command: format!("bb thread tell {THREAD} hi"),
            status: TimelineRowStatus::Completed,
            exit_code: Some(0),
            ..CommandWorkRow::default()
        };
        assert_eq!(parse_sent_thread_message(&row), expected("hi"));
        row.exit_code = Some(1);
        assert_eq!(parse_sent_thread_message(&row), None);
        row.exit_code = None;
        row.status = TimelineRowStatus::Pending;
        assert_eq!(parse_sent_thread_message(&row), None);
    }
}
