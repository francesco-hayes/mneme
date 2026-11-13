//! Reads and rewrites `notes.md` without disturbing what it does not own.
//!
//! This is a small purpose-built parser, not a YAML library, and that is
//! deliberate. A YAML library reads the block into a value tree and writes a
//! fresh block back, which drops comments, reorders or restyles keys, and
//! reformats lists a person or another tool wrote by hand. Mneme only ever
//! reads and writes a handful of one-line `key: value` entries, so this
//! module works on lines instead:
//!
//! - The file is split into the opening `---` line, the entries, the closing
//!   `---` line and the body. Only the first closing `---` counts, so `---`
//!   lines in the body are ordinary text.
//! - Each top-level `key:` line starts an entry; the lines under it
//!   (indented values, list items, comments, blank lines) belong to it and
//!   are kept as raw text.
//! - Setting a key rewrites that one line. Every other byte of the file,
//!   unknown keys and their order included, is written back as it was read.
//!   A file that is parsed and rendered without a change is byte-identical.
//!
//! Values of the keys Mneme reads must be one-line scalars: plain, 'single
//! quoted' or "double quoted". What is written is valid YAML; the tests check
//! that with a real YAML parser.

use crate::error::FrontmatterError;

/// A parsed `notes.md`: frontmatter entries plus the body, both kept exactly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NotesFile {
    /// Whether the file started with a UTF-8 byte order mark.
    bom: bool,
    /// The opening `---` line with its line ending.
    open: String,
    /// Comment and blank lines before the first key.
    preamble: String,
    entries: Vec<Entry>,
    /// The closing `---` line with its line ending, if it had one.
    close: String,
    /// Everything after the closing line, untouched.
    pub(crate) body: String,
}

/// One top-level key with everything that belongs to it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    key: String,
    /// The `key: value` line with its line ending.
    line: String,
    /// Byte offset in `line` just past the colon.
    value_start: usize,
    /// The lines that follow, up to the next key: the rest of a multi-line
    /// value, comments, blank lines. Raw, with line endings.
    rest: String,
}

impl Entry {
    /// The text after the colon, without the line ending.
    fn value_text(&self) -> &str {
        without_line_ending(&self.line[self.value_start..])
    }
}

/// A one-line YAML scalar, unquoted and unescaped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Scalar {
    pub(crate) text: String,
    /// Whether it was written in quotes. Quoted text is always a string;
    /// plain text can also mean null.
    pub(crate) quoted: bool,
}

impl Scalar {
    /// True for an empty value, `~` and `null`, as in YAML.
    pub(crate) fn is_null(&self) -> bool {
        !self.quoted && matches!(self.text.as_str(), "" | "~" | "null" | "Null" | "NULL")
    }
}

impl NotesFile {
    /// An empty frontmatter block with an empty body.
    pub(crate) fn new() -> Self {
        Self {
            bom: false,
            open: "---\n".to_owned(),
            preamble: String::new(),
            entries: Vec::new(),
            close: "---\n".to_owned(),
            body: String::new(),
        }
    }

    /// Splits the text of a `notes.md` into frontmatter entries and body.
    pub(crate) fn parse(text: &str) -> Result<Self, FrontmatterError> {
        let (bom, text) = match text.strip_prefix('\u{FEFF}') {
            Some(rest) => (true, rest),
            None => (false, text),
        };

        let mut lines = text.split_inclusive('\n');
        let open = match lines.next() {
            Some(line) if is_delimiter(line) => line,
            _ => return Err(FrontmatterError::Missing),
        };

        let mut offset = open.len();
        let mut preamble = String::new();
        let mut entries: Vec<Entry> = Vec::new();

        for (index, line) in lines.enumerate() {
            offset += line.len();
            if is_delimiter(line) {
                return Ok(Self {
                    bom,
                    open: open.to_owned(),
                    preamble,
                    entries,
                    close: line.to_owned(),
                    body: text[offset..].to_owned(),
                });
            }

            // The opening delimiter is line 1.
            let invalid = FrontmatterError::InvalidLine { line: index + 2 };
            let content = without_line_ending(line);
            match classify(content) {
                LineKind::Trivia => match entries.last_mut() {
                    Some(entry) => entry.rest.push_str(line),
                    None => preamble.push_str(line),
                },
                LineKind::Continuation => match entries.last_mut() {
                    Some(entry) => entry.rest.push_str(line),
                    None => return Err(invalid),
                },
                LineKind::Key => {
                    let Some((key, value_start)) = split_key(content) else {
                        return Err(invalid);
                    };
                    if entries.iter().any(|entry| entry.key == key) {
                        return Err(FrontmatterError::DuplicateKey { key });
                    }
                    entries.push(Entry {
                        key,
                        line: line.to_owned(),
                        value_start,
                        rest: String::new(),
                    });
                }
            }
        }

        Err(FrontmatterError::Unterminated)
    }

    /// The full text of the file.
    pub(crate) fn render(&self) -> String {
        let mut out = String::with_capacity(self.body.len() + 512);
        if self.bom {
            out.push('\u{FEFF}');
        }
        out.push_str(&self.open);
        out.push_str(&self.preamble);
        for entry in &self.entries {
            out.push_str(&entry.line);
            out.push_str(&entry.rest);
        }
        out.push_str(&self.close);
        if !self.body.is_empty() && !self.close.ends_with('\n') {
            // The file ended right after `---`; a body needs its own line.
            out.push_str(self.newline());
        }
        out.push_str(&self.body);
        out
    }

    /// The value of `key` as a one-line scalar. `Ok(None)` when the key is
    /// not there; an error when its value is a list, a map, a block scalar or
    /// malformed.
    pub(crate) fn get(&self, key: &str) -> Result<Option<Scalar>, FrontmatterError> {
        let Some(entry) = self.entries.iter().find(|entry| entry.key == key) else {
            return Ok(None);
        };
        let invalid = |reason: &str| FrontmatterError::InvalidValue {
            key: key.to_owned(),
            reason: reason.to_owned(),
        };
        let (scalar, _comment) = parse_scalar(entry.value_text()).map_err(invalid)?;
        if entry.rest.lines().any(|line| !is_trivia(line)) {
            return Err(invalid("must be a single-line value"));
        }
        Ok(Some(scalar))
    }

    /// Sets `key` to `value`, which must already be YAML scalar text (see
    /// [`format_string`] and [`quote`]); an empty `value` writes a null.
    ///
    /// An existing key keeps its place, its spelling and any trailing
    /// comment; a new key goes last. If the key already holds this value the
    /// file is left exactly as it is.
    pub(crate) fn set(&mut self, key: &str, value: &str) {
        let newline = self.newline();
        let Some(entry) = self.entries.iter_mut().find(|entry| entry.key == key) else {
            let mut line = format!("{key}:");
            if !value.is_empty() {
                line.push(' ');
                line.push_str(value);
            }
            line.push_str(newline);
            self.entries.push(Entry {
                key: key.to_owned(),
                line,
                value_start: key.len() + 1,
                rest: String::new(),
            });
            return;
        };

        let current = parse_scalar(entry.value_text()).ok();
        let one_line = entry.rest.lines().all(is_kept_on_rewrite);
        let wanted = parse_scalar(value).ok().map(|(scalar, _)| scalar);
        if one_line
            && let (Some((current, _)), Some(wanted)) = (&current, &wanted)
            && same_value(current, wanted)
        {
            return;
        }

        let comment = current.and_then(|(_, comment)| comment.map(str::to_owned));
        let ending = entry.line[without_line_ending(&entry.line).len()..].to_owned();
        let mut line = entry.line[..entry.value_start].to_owned();
        if !value.is_empty() {
            line.push(' ');
            line.push_str(value);
        }
        if let Some(comment) = comment {
            line.push(' ');
            line.push_str(&comment);
        }
        line.push_str(if ending.is_empty() { newline } else { &ending });

        entry.line = line;
        // Lines that were the rest of the old value go; comments and blank
        // lines that merely sit below the key stay.
        entry.rest = entry
            .rest
            .split_inclusive('\n')
            .filter(|line| is_kept_on_rewrite(line))
            .collect();
    }

    /// Sets `key` to a string, quoted only when YAML needs it.
    pub(crate) fn set_string(&mut self, key: &str, value: &str) {
        self.set(key, &format_string(value));
    }

    /// The line ending this file uses, taken from its first line.
    fn newline(&self) -> &'static str {
        if self.open.ends_with("\r\n") {
            "\r\n"
        } else {
            "\n"
        }
    }
}

enum LineKind {
    /// Blank or a comment.
    Trivia,
    /// More of the value above: indented text, or a list item under a key.
    Continuation,
    /// Anything else at the left margin; must be `key: value`.
    Key,
}

fn classify(content: &str) -> LineKind {
    if is_trivia(content) {
        LineKind::Trivia
    } else if content.starts_with([' ', '\t']) || content == "-" || content.starts_with("- ") {
        LineKind::Continuation
    } else {
        LineKind::Key
    }
}

fn is_delimiter(line: &str) -> bool {
    line.trim_end() == "---"
}

fn is_trivia(line: &str) -> bool {
    let trimmed = line.trim_start_matches([' ', '\t']);
    let trimmed = without_line_ending(trimmed);
    trimmed.is_empty() || trimmed.starts_with('#')
}

/// Whether a line under a key survives when that key's value is replaced:
/// blank lines and comments at the left margin do, anything indented was part
/// of the old value.
fn is_kept_on_rewrite(line: &str) -> bool {
    let content = without_line_ending(line);
    content.trim().is_empty() || content.starts_with('#')
}

fn without_line_ending(line: &str) -> &str {
    let line = line.strip_suffix('\n').unwrap_or(line);
    line.strip_suffix('\r').unwrap_or(line)
}

/// Splits `key: value` into the key and the byte offset just past the colon.
/// The colon must be followed by a space, a tab or the end of the line, as
/// YAML requires. A key may be wrapped in quotes.
fn split_key(content: &str) -> Option<(String, usize)> {
    let (key, colon) = match content.chars().next() {
        Some(quote @ ('"' | '\'')) => {
            let end = 1 + content[1..].find(quote)?;
            let after = &content[end + 1..];
            let spaces = after.len() - after.trim_start_matches(' ').len();
            (&content[1..end], end + 1 + spaces)
        }
        _ => {
            let colon = content
                .match_indices(':')
                .map(|(index, _)| index)
                .find(|&index| {
                    matches!(content[index + 1..].chars().next(), None | Some(' ' | '\t'))
                })?;
            (content[..colon].trim_end(), colon)
        }
    };

    let after_colon = content[colon..].strip_prefix(':')?;
    let ends_key = matches!(after_colon.chars().next(), None | Some(' ' | '\t'));
    (!key.is_empty() && ends_key).then(|| (key.to_owned(), colon + 1))
}

/// Parses the text after `key:` as a one-line scalar. Returns the value and
/// the trailing `# comment`, if any. The error completes the sentence
/// "key `x` ...".
fn parse_scalar(raw: &str) -> Result<(Scalar, Option<&str>), &'static str> {
    let text = raw.trim_start_matches([' ', '\t']);
    match text.chars().next() {
        None => Ok((plain(""), None)),
        Some('#') => Ok((plain(""), Some(text))),
        Some('"') => {
            let (value, rest) = parse_double_quoted(&text[1..])?;
            Ok((quoted(value), trailing_comment(rest)?))
        }
        Some('\'') => {
            let (value, rest) = parse_single_quoted(&text[1..])?;
            Ok((quoted(value), trailing_comment(rest)?))
        }
        Some('|' | '>') => Err("uses a block scalar; write the value on one line"),
        Some('[' | '{') => Err("must be a single value, not a list or a map"),
        Some('&' | '*' | '!') => Err("uses a YAML anchor, alias or tag, which is not supported"),
        Some(_) => {
            let comment_at = text
                .match_indices('#')
                .map(|(index, _)| index)
                .find(|&index| text[..index].ends_with([' ', '\t']));
            match comment_at {
                Some(index) => Ok((plain(text[..index].trim_end()), Some(&text[index..]))),
                None => Ok((plain(text.trim_end()), None)),
            }
        }
    }
}

fn plain(text: &str) -> Scalar {
    Scalar {
        text: text.to_owned(),
        quoted: false,
    }
}

fn quoted(text: String) -> Scalar {
    Scalar { text, quoted: true }
}

/// After a closing quote only whitespace and a comment may follow.
fn trailing_comment(rest: &str) -> Result<Option<&str>, &'static str> {
    let rest = rest.trim_start_matches([' ', '\t']);
    if rest.is_empty() {
        Ok(None)
    } else if rest.starts_with('#') {
        Ok(Some(rest))
    } else {
        Err("has text after its closing quote")
    }
}

/// Parses the inside of a `'single quoted'` scalar, where `''` is a quote.
/// Returns the value and what follows the closing quote.
fn parse_single_quoted(text: &str) -> Result<(String, &str), &'static str> {
    let mut value = String::new();
    let mut chars = text.char_indices();
    while let Some((index, c)) = chars.next() {
        if c != '\'' {
            value.push(c);
        } else if text[index + 1..].starts_with('\'') {
            value.push('\'');
            chars.next();
        } else {
            return Ok((value, &text[index + 1..]));
        }
    }
    Err("has a quote that is never closed")
}

/// Parses the inside of a `"double quoted"` scalar with YAML's escapes.
/// Returns the value and what follows the closing quote.
fn parse_double_quoted(text: &str) -> Result<(String, &str), &'static str> {
    const BAD_ESCAPE: &str = "has an escape sequence that is not valid YAML";

    let mut value = String::new();
    let mut chars = text.char_indices();
    while let Some((index, c)) = chars.next() {
        match c {
            '"' => return Ok((value, &text[index + 1..])),
            '\\' => {
                let (_, escape) = chars.next().ok_or(BAD_ESCAPE)?;
                let hex_digits = match escape {
                    'x' => 2,
                    'u' => 4,
                    'U' => 8,
                    _ => 0,
                };
                if hex_digits > 0 {
                    let mut code = 0u32;
                    for _ in 0..hex_digits {
                        let (_, digit) = chars.next().ok_or(BAD_ESCAPE)?;
                        code = code * 16 + digit.to_digit(16).ok_or(BAD_ESCAPE)?;
                    }
                    value.push(char::from_u32(code).ok_or(BAD_ESCAPE)?);
                    continue;
                }
                value.push(match escape {
                    '0' => '\0',
                    'a' => '\u{07}',
                    'b' => '\u{08}',
                    't' | '\t' => '\t',
                    'n' => '\n',
                    'v' => '\u{0B}',
                    'f' => '\u{0C}',
                    'r' => '\r',
                    'e' => '\u{1B}',
                    ' ' => ' ',
                    '"' => '"',
                    '/' => '/',
                    '\\' => '\\',
                    'N' => '\u{85}',
                    '_' => '\u{A0}',
                    'L' => '\u{2028}',
                    'P' => '\u{2029}',
                    _ => return Err(BAD_ESCAPE),
                });
            }
            c => value.push(c),
        }
    }
    Err("has a quote that is never closed")
}

/// Whether two scalars mean the same thing: both null, or the same text.
fn same_value(a: &Scalar, b: &Scalar) -> bool {
    match (a.is_null(), b.is_null()) {
        (true, true) => true,
        (false, false) => a.text == b.text,
        _ => false,
    }
}

/// Formats a string as a YAML scalar: plain when that is unambiguous,
/// otherwise double-quoted.
pub(crate) fn format_string(value: &str) -> String {
    if is_plain_safe(value) {
        value.to_owned()
    } else {
        quote(value)
    }
}

/// Formats a string as a double-quoted YAML scalar. The escapes used are the
/// ones YAML shares with JSON.
pub(crate) fn quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}' | '\u{FEFF}') => {
                out.push_str(&format!("\\u{:04X}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Whether `value` can be written without quotes and still be read back as
/// this exact string by any YAML parser, 1.1 or 1.2.
///
/// The rule is stricter than YAML's on purpose. A plain scalar must start
/// with a letter, which rules out every indicator character and everything
/// that could resolve to a number or a date. It must not be a word YAML 1.1
/// reads as a boolean or null (`no` is a language code here, not `false`),
/// and must not contain `: ` or ` #`, which would end the value early.
fn is_plain_safe(value: &str) -> bool {
    const RESERVED: [&str; 9] = ["null", "true", "false", "yes", "no", "on", "off", "y", "n"];

    let Some(first) = value.chars().next() else {
        return false;
    };
    first.is_alphabetic()
        && !value.ends_with(|c: char| c.is_whitespace() || c == ':')
        && !value
            .chars()
            .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}' | '\u{FEFF}'))
        && !value.contains(": ")
        && !value.contains(":\t")
        && !value.contains(" #")
        && !value.contains("\t#")
        && !RESERVED.iter().any(|word| value.eq_ignore_ascii_case(word))
}

#[cfg(test)]
mod tests {
    use saphyr::{LoadableYamlNode, Yaml};

    use super::*;

    /// A hand-edited file: comments, unknown keys of every shape, odd
    /// spacing, and a body that uses `---` as a horizontal rule.
    const HAND_EDITED: &str = "\
---
# written by hand
id: 7b1c2f9e-5a34-4c1d-9e0f-2a6b8c4d1e3f
title: \"Q4: planning\"   # the big one
tags:
  - work
  - planning
title_edited: true

aliases: [kickoff, \"q4\"]
date: \"2026-10-27T14:30:00+01:00\"
duration: 1800
summary: |
  First line.
  ---
  Still the summary.
status: complete
'quoted key': yes
attendees:
- Ana
- Ben
model: large-v3-turbo
language: en
x-future:
  nested:
    deep: 1
---
# Notes

Agenda

---

- one
---
- two
";

    fn value(doc: &NotesFile, key: &str) -> String {
        doc.get(key).unwrap().unwrap().text
    }

    fn yaml_of(doc: &NotesFile) -> String {
        let text = doc.render();
        let block = text.strip_prefix("---\n").unwrap();
        block[..block.find("\n---\n").unwrap() + 1].to_owned()
    }

    #[test]
    fn parse_then_render_is_byte_identical() {
        for text in [
            HAND_EDITED,
            "---\n---\n",
            "---\n---",
            "---\nid: a\n---",
            "---\nid: a\n---\nbody without final newline",
            "---\r\nid: a\r\ntitle: Windows\r\n---\r\nbody\r\n---\r\nmore\r\n",
            "\u{FEFF}---\nid: a\n---\nbody\n",
            "---   \n# only a comment\n\n---\t\n\n\nbody\n",
            "---\nkey:\n\tindented with a tab\n---\n",
            "---\na:    spaced   \nb:\t tabbed\n\"c\" : quoted key\n---\n---\n---\n",
        ] {
            let doc = NotesFile::parse(text).unwrap();
            assert_eq!(doc.render(), text);
        }
    }

    #[test]
    fn the_body_starts_after_the_first_closing_delimiter() {
        let doc = NotesFile::parse(HAND_EDITED).unwrap();
        assert_eq!(doc.body, "# Notes\n\nAgenda\n\n---\n\n- one\n---\n- two\n");

        let doc = NotesFile::parse("---\nid: a\n---\n---\nstill body\n---\n").unwrap();
        assert_eq!(doc.body, "---\nstill body\n---\n");
    }

    #[test]
    fn reads_plain_and_quoted_values() {
        let doc = NotesFile::parse(HAND_EDITED).unwrap();

        assert_eq!(value(&doc, "id"), "7b1c2f9e-5a34-4c1d-9e0f-2a6b8c4d1e3f");
        assert_eq!(value(&doc, "title"), "Q4: planning");
        assert_eq!(value(&doc, "title_edited"), "true");
        assert_eq!(value(&doc, "date"), "2026-10-27T14:30:00+01:00");
        assert_eq!(value(&doc, "duration"), "1800");
        assert_eq!(value(&doc, "status"), "complete");
        assert_eq!(value(&doc, "quoted key"), "yes");
        assert_eq!(doc.get("not there"), Ok(None));
    }

    #[test]
    fn values_that_are_not_one_line_scalars_are_errors_for_that_key_only() {
        let doc = NotesFile::parse(HAND_EDITED).unwrap();

        for key in ["tags", "aliases", "summary", "attendees", "x-future"] {
            let err = doc.get(key).unwrap_err();
            assert!(
                matches!(&err, FrontmatterError::InvalidValue { key: k, .. } if k == key),
                "{key}: {err:?}"
            );
        }
        assert_eq!(value(&doc, "status"), "complete");
    }

    #[test]
    fn scalars_follow_yaml_quoting_rules() {
        let cases: [(&str, &str, bool); 16] = [
            ("", "", false),
            ("   ", "", false),
            (" # only a comment", "", false),
            (" plain text ", "plain text", false),
            (" value # comment", "value", false),
            (" c#, f# and more", "c#, f# and more", false),
            (" a: b", "a: b", false),
            (" 'single'", "single", true),
            (" 'it''s'  # note", "it's", true),
            (" ''", "", true),
            (r#" "double""#, "double", true),
            (r#" "a \"b\" \\ c""#, r#"a "b" \ c"#, true),
            (r#" "tab\there\nnewline""#, "tab\there\nnewline", true),
            (r#" "\x41é\U0001F680""#, "Aé🚀", true),
            (r##" "# not a comment""##, "# not a comment", true),
            ("\t~", "~", false),
        ];
        for (raw, text, quoted) in cases {
            let (scalar, _) = parse_scalar(raw).unwrap();
            assert_eq!(
                (scalar.text.as_str(), scalar.quoted),
                (text, quoted),
                "{raw:?}"
            );
        }

        for raw in [
            " \"open",
            " 'open",
            " \"closed\" trailing",
            r#" "bad \q escape""#,
            r#" "\ud83d""#,
            r#" "\x4""#,
            " | ",
            " >-",
            " [a, b]",
            " {a: b}",
            " &anchor value",
            " *alias",
            " !!str value",
        ] {
            assert!(parse_scalar(raw).is_err(), "{raw:?}");
        }
    }

    #[test]
    fn null_is_empty_tilde_or_the_word_but_never_a_quoted_string() {
        for raw in ["", " ", " ~", " null", " Null", " NULL", " # nothing"] {
            assert!(parse_scalar(raw).unwrap().0.is_null(), "{raw:?}");
        }
        for raw in [" ''", " \"\"", " \"null\"", " '~'", " nil", " none", " 0"] {
            assert!(!parse_scalar(raw).unwrap().0.is_null(), "{raw:?}");
        }
    }

    #[test]
    fn setting_a_key_rewrites_only_its_line() {
        let mut doc = NotesFile::parse(HAND_EDITED).unwrap();

        doc.set("status", "interrupted");
        doc.set("duration", "2400");

        let expected = HAND_EDITED
            .replace("status: complete\n", "status: interrupted\n")
            .replace("duration: 1800\n", "duration: 2400\n");
        assert_eq!(doc.render(), expected);
    }

    #[test]
    fn setting_a_key_keeps_its_spelling_and_trailing_comment() {
        let mut doc = NotesFile::parse(HAND_EDITED).unwrap();

        doc.set_string("title", "Retro");
        doc.set_string("quoted key", "no");

        let expected = HAND_EDITED
            .replace(
                "title: \"Q4: planning\"   # the big one\n",
                "title: Retro # the big one\n",
            )
            .replace("'quoted key': yes\n", "'quoted key': \"no\"\n");
        assert_eq!(doc.render(), expected);
    }

    #[test]
    fn setting_the_value_a_key_already_has_changes_nothing() {
        let mut doc = NotesFile::parse(HAND_EDITED).unwrap();

        doc.set_string("title", "Q4: planning");
        doc.set("title_edited", "true");
        doc.set("date", &quote("2026-10-27T14:30:00+01:00"));
        doc.set("duration", "1800");
        doc.set("status", "complete");

        assert_eq!(doc.render(), HAND_EDITED);

        let text = "---\nduration:   # not yet\nmodel: ~\n---\n";
        let mut doc = NotesFile::parse(text).unwrap();
        doc.set("duration", "");
        doc.set("model", "");
        assert_eq!(doc.render(), text);
    }

    #[test]
    fn a_new_key_is_added_last_and_null_is_written_empty() {
        let mut doc = NotesFile::parse("---\nid: a\n# trailing comment\n---\nbody\n").unwrap();

        doc.set("duration", "");
        doc.set("status", "recording");

        assert_eq!(
            doc.render(),
            "---\nid: a\n# trailing comment\nduration:\nstatus: recording\n---\nbody\n"
        );
    }

    #[test]
    fn replacing_a_multi_line_value_drops_its_old_lines_but_not_comments() {
        let text = "---\ntitle: >\n  folded\n  title\n\n# about the date\ndate: x\n---\n";
        let mut doc = NotesFile::parse(text).unwrap();
        assert!(doc.get("title").is_err());

        doc.set_string("title", "One line");

        assert_eq!(
            doc.render(),
            "---\ntitle: One line\n\n# about the date\ndate: x\n---\n"
        );
        assert_eq!(value(&doc, "title"), "One line");
    }

    #[test]
    fn new_lines_use_the_line_ending_of_the_file() {
        let mut doc = NotesFile::parse("---\r\nid: a\r\n---\r\nbody\r\n").unwrap();

        doc.set("id", "b");
        doc.set("status", "complete");

        assert_eq!(
            doc.render(),
            "---\r\nid: b\r\nstatus: complete\r\n---\r\nbody\r\n"
        );
    }

    #[test]
    fn a_body_can_be_added_to_a_file_that_ended_at_the_delimiter() {
        let mut doc = NotesFile::parse("---\nid: a\n---").unwrap();
        assert_eq!(doc.body, "");

        doc.body.push_str("first note\n");

        assert_eq!(doc.render(), "---\nid: a\n---\nfirst note\n");
    }

    #[test]
    fn files_without_usable_frontmatter_are_errors() {
        use FrontmatterError::*;

        let cases = [
            ("", Missing),
            ("just notes\n", Missing),
            ("\n---\nid: a\n---\n", Missing),
            ("# Heading\n---\nid: a\n---\n", Missing),
            ("----\nid: a\n----\n", Missing),
            ("---", Unterminated),
            ("---\nid: a\ntitle: never closed\n", Unterminated),
            ("---\nid: a\n ---\n", Unterminated),
            ("---\nid: a\nnot a key\n---\n", InvalidLine { line: 3 }),
            ("---\n  indented: first\n---\n", InvalidLine { line: 2 }),
            ("---\n- a\n- list\n---\n", InvalidLine { line: 2 }),
            ("---\nid:a\n---\n", InvalidLine { line: 2 }),
            ("---\n: no key\n---\n", InvalidLine { line: 2 }),
            ("---\n\"unclosed: x\n---\n", InvalidLine { line: 2 }),
            (
                "---\nid: a\nid: b\n---\n",
                DuplicateKey {
                    key: "id".to_owned(),
                },
            ),
        ];
        for (text, expected) in cases {
            assert_eq!(NotesFile::parse(text), Err(expected), "{text:?}");
        }
    }

    /// Strings chosen to break naive YAML writers.
    const NASTY: &[&str] = &[
        "Meeting, Oct 27 2:30 PM",
        "Standup",
        "Q4: planning",
        "ends with colon:",
        "has # hash",
        "c# and f#",
        "#leading hash",
        "- leading dash",
        "? question",
        "[brackets]",
        "{braces}",
        "a, b, [c] {d}",
        "\"double quoted\"",
        "'single quoted'",
        "it's \"both\"",
        "back\\slash",
        "@at and `backtick`",
        "%percent",
        "&anchor",
        "*alias",
        "!tag",
        "|pipe",
        ">folded",
        "yes",
        "No",
        "NULL",
        "true",
        "~",
        "123",
        "1.5e3",
        "0x1F",
        "2026-10-27",
        "12:30:00",
        " leading and trailing ",
        "tab\tinside",
        "two\nlines",
        "control \u{7} \u{1b} \u{85} \u{2028}",
        "Réunion d'équipe",
        "設計レビュー 🚀",
        "",
    ];

    #[test]
    fn any_string_survives_being_written_and_read_back() {
        for &text in NASTY {
            let mut doc = NotesFile::new();
            doc.set_string("title", text);
            doc.set("after", "1");

            let reread = NotesFile::parse(&doc.render()).unwrap();
            let scalar = reread.get("title").unwrap().unwrap();
            assert_eq!(scalar.text, text, "{:?}", doc.render());
            assert!(!scalar.is_null(), "{text:?}");
            assert_eq!(value(&reread, "after"), "1");
        }
    }

    #[test]
    fn a_real_yaml_parser_reads_every_written_string_as_that_string() {
        for &text in NASTY {
            let mut doc = NotesFile::new();
            doc.set_string("title", text);
            doc.set("after", "1");

            let yaml = yaml_of(&doc);
            let docs = Yaml::load_from_str(&yaml).unwrap_or_else(|err| {
                panic!("invalid YAML for {text:?}: {err}\n{yaml}");
            });
            assert_eq!(docs.len(), 1, "{yaml}");
            assert_eq!(docs[0]["title"].as_str(), Some(text), "{yaml}");
            assert_eq!(docs[0]["after"].as_integer(), Some(1), "{yaml}");
        }
    }

    #[test]
    fn strings_are_quoted_only_when_they_need_it() {
        for (text, written) in [
            ("Standup", "Standup"),
            ("Meeting, Oct 27 2:30 PM", "Meeting, Oct 27 2:30 PM"),
            ("large-v3-turbo", "large-v3-turbo"),
            ("auto", "auto"),
            ("en", "en"),
            ("Réunion d'équipe", "Réunion d'équipe"),
            ("no", "\"no\""),
            ("Yes", "\"Yes\""),
            ("null", "\"null\""),
            ("Q4: planning", "\"Q4: planning\""),
            ("2026 planning", "\"2026 planning\""),
            ("say \"hi\"", "say \"hi\""),
            ("\"hi\"", "\"\\\"hi\\\"\""),
            ("a\\b\nc", "\"a\\\\b\\nc\""),
            ("bell\u{7}", "\"bell\\u0007\""),
            ("", "\"\""),
        ] {
            assert_eq!(format_string(text), written, "{text:?}");
        }
    }

    #[test]
    fn the_hand_edited_sample_is_valid_yaml_before_and_after_a_rewrite() {
        let mut doc = NotesFile::parse(HAND_EDITED).unwrap();
        doc.set_string("title", "Renamed: again");
        doc.set("status", "interrupted");

        let docs = Yaml::load_from_str(&yaml_of(&doc)).unwrap();
        let yaml = &docs[0];
        assert_eq!(yaml["title"].as_str(), Some("Renamed: again"));
        assert_eq!(yaml["status"].as_str(), Some("interrupted"));
        assert_eq!(yaml["tags"][1].as_str(), Some("planning"));
        assert_eq!(yaml["attendees"][0].as_str(), Some("Ana"));
        assert_eq!(yaml["x-future"]["nested"]["deep"].as_integer(), Some(1));
        assert_eq!(
            yaml["summary"].as_str(),
            Some("First line.\n---\nStill the summary.\n")
        );
    }
}
