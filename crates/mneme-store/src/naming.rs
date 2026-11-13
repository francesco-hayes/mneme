//! Folder names: `YYYY-MM-DD HHMM Title`.

use crate::time::StartTime;

/// The title part of the folder of a meeting that keeps its default title.
pub const DEFAULT_FOLDER_TITLE: &str = "Meeting";

/// Longest title part of a folder name, in characters.
const MAX_TITLE_CHARS: usize = 80;

/// Longest title part of a folder name, in UTF-8 bytes. A file name on APFS
/// holds 255 bytes; the date prefix takes 16, the rest is headroom.
const MAX_TITLE_BYTES: usize = 200;

/// Turns a meeting title into the title part of a folder name.
///
/// - Path separators (`/`, `\`) and the other characters that file systems
///   or Finder treat specially (`:`, `*`, `?`, `"`, `<`, `>`, `|`) are taken
///   out. Each leaves a word break, so `Q3/Q4 review` becomes `Q3 Q4 review`.
/// - Control characters and invisible text-direction marks are removed.
/// - Runs of whitespace, including tabs and newlines, become one space.
/// - Leading dots and spaces go, so the folder is never hidden and never
///   `..`. Trailing dots and spaces go too.
/// - At most 80 characters and 200 bytes are kept.
/// - If nothing is left, the result is `Meeting`.
///
/// The display title in frontmatter is not affected by any of this.
pub fn sanitize_title(title: &str) -> String {
    let cleaned = clean(title, true);
    let trimmed = cleaned.trim_start_matches(['.', ' ']);
    let capped = truncate(trimmed, MAX_TITLE_CHARS, MAX_TITLE_BYTES);
    let result = capped.trim_end_matches(['.', ' ']);
    if result.is_empty() {
        DEFAULT_FOLDER_TITLE.to_owned()
    } else {
        result.to_owned()
    }
}

/// Tidies a title for display and frontmatter: one line, no control
/// characters, single spaces, no surrounding whitespace. Punctuation is kept.
/// Empty when the input has no visible text.
pub(crate) fn normalize_title(title: &str) -> String {
    clean(title, false)
}

/// The folder name for a meeting that started at `start`. `title_part` must
/// already be sanitized.
pub(crate) fn folder_name(start: &StartTime, title_part: &str) -> String {
    format!("{} {}", start.folder_prefix(), title_part)
}

/// Drops control and direction characters and collapses whitespace runs to
/// one space, trimming both ends. With `for_file_name`, characters unsafe in
/// file names count as whitespace.
fn clean(title: &str, for_file_name: bool) -> String {
    let mut out = String::with_capacity(title.len());
    let mut pending_space = false;
    for c in title.chars() {
        if c.is_whitespace() || (for_file_name && is_unsafe_in_file_names(c)) {
            pending_space = true;
        } else if c.is_control() || is_direction_mark(c) {
            // Removed without leaving a gap.
        } else {
            if pending_space && !out.is_empty() {
                out.push(' ');
            }
            pending_space = false;
            out.push(c);
        }
    }
    out
}

fn is_unsafe_in_file_names(c: char) -> bool {
    matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
}

/// Invisible characters that reorder how text is displayed and so can make a
/// file name read as something it is not, plus the byte order mark.
fn is_direction_mark(c: char) -> bool {
    matches!(
        c,
        '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}'
    )
}

/// The longest prefix of `s` within both limits, cut between characters.
fn truncate(s: &str, max_chars: usize, max_bytes: usize) -> &str {
    let mut end = 0;
    for (count, (index, c)) in s.char_indices().enumerate() {
        let next = index + c.len_utf8();
        if count == max_chars || next > max_bytes {
            break;
        }
        end = next;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_titles_pass_through() {
        for title in [
            "Standup",
            "Weekly sync with Ana",
            "Q4 planning (draft 2)",
            "Budget review - final, v3",
            "Réunion d'équipe",
            "設計レビュー",
            "Launch 🚀 retro",
            "C++ & Rust, 50% each!",
        ] {
            assert_eq!(sanitize_title(title), title);
        }
    }

    #[test]
    fn path_separators_and_reserved_characters_become_word_breaks() {
        assert_eq!(sanitize_title("Q3/Q4 review"), "Q3 Q4 review");
        assert_eq!(sanitize_title(r"a\b"), "a b");
        assert_eq!(sanitize_title("Client: kickoff"), "Client kickoff");
        assert_eq!(sanitize_title("Sync 2:30"), "Sync 2 30");
        assert_eq!(sanitize_title(r#"a*b?c"d<e>f|g"#), "a b c d e f g");
        assert_eq!(sanitize_title("///"), "Meeting");
    }

    #[test]
    fn a_title_cannot_climb_out_of_the_root() {
        for (title, expected) in [
            ("../../etc/passwd", "etc passwd"),
            ("..", "Meeting"),
            (".", "Meeting"),
            ("/", "Meeting"),
            ("/absolute/path", "absolute path"),
            (r"..\..\windows", "windows"),
            ("~/Documents", "~ Documents"),
            ("a/../b", "a .. b"),
        ] {
            let name = sanitize_title(title);
            assert_eq!(name, expected, "{title:?}");
            assert!(!name.contains(['/', '\\']), "{title:?}");
        }
    }

    #[test]
    fn leading_dots_and_trailing_dots_and_spaces_are_removed() {
        assert_eq!(sanitize_title(".hidden"), "hidden");
        assert_eq!(sanitize_title("... . .env"), "env");
        assert_eq!(sanitize_title("Wrap up..."), "Wrap up");
        assert_eq!(sanitize_title("  padded  "), "padded");
        assert_eq!(sanitize_title("ends with dot. "), "ends with dot");
        assert_eq!(sanitize_title("v1.2 notes"), "v1.2 notes");
    }

    #[test]
    fn whitespace_is_collapsed_and_control_characters_are_removed() {
        assert_eq!(sanitize_title("a \t\n\r  b"), "a b");
        assert_eq!(sanitize_title("line one\nline two"), "line one line two");
        assert_eq!(sanitize_title("nul\0bell\u{7}esc\u{1b}"), "nulbellesc");
        assert_eq!(sanitize_title("del\u{7f}c1\u{85}\u{9b}end"), "delc1 end");
        assert_eq!(sanitize_title("no\u{a0}break\u{2028}line"), "no break line");
    }

    #[test]
    fn direction_overrides_are_removed() {
        // U+202E makes "gpj.exe" display as "exe.jpg" in some file browsers.
        assert_eq!(sanitize_title("photo\u{202E}gpj.exe"), "photogpj.exe");
        assert_eq!(sanitize_title("\u{FEFF}\u{200F}title\u{2066}"), "title");
        // Joiners that scripts and emoji need are kept.
        assert_eq!(sanitize_title("می\u{200C}خواهم"), "می\u{200C}خواهم");
        assert_eq!(
            sanitize_title("👩\u{200D}💻 pairing"),
            "👩\u{200D}💻 pairing"
        );
    }

    #[test]
    fn empty_results_fall_back_to_meeting() {
        for title in ["", " ", "\t\n", "...", ". . .", "\0", "?:*", "\u{202E}"] {
            assert_eq!(sanitize_title(title), "Meeting", "{title:?}");
        }
    }

    #[test]
    fn long_titles_are_capped_by_characters_and_by_bytes() {
        let ascii = sanitize_title(&"a".repeat(500));
        assert_eq!(ascii.chars().count(), 80);

        // 4 bytes each: the byte cap bites first, on a character boundary.
        let emoji = sanitize_title(&"🚀".repeat(500));
        assert_eq!(emoji.len(), 200);
        assert_eq!(emoji.chars().count(), 50);

        // 3 bytes each: 66 characters fit in 200 bytes.
        let cjk = sanitize_title(&"設".repeat(500));
        assert_eq!(cjk.chars().count(), 66);
        assert!(cjk.len() <= 200);
    }

    #[test]
    fn a_cut_that_lands_after_a_space_or_dot_is_trimmed_again() {
        let title = format!("{} tail", "a".repeat(79));
        assert_eq!(sanitize_title(&title), "a".repeat(79));

        let title = format!("{}...more", "b".repeat(78));
        assert_eq!(sanitize_title(&title), "b".repeat(78));
    }

    #[test]
    fn sanitizing_twice_changes_nothing() {
        for title in [
            "Q3/Q4 review",
            "../../etc/passwd",
            "  .hidden.  ",
            "a \t b",
            "",
            &"x y. ".repeat(100),
        ] {
            let once = sanitize_title(title);
            assert_eq!(sanitize_title(&once), once, "{title:?}");
        }
    }

    #[test]
    fn display_titles_keep_punctuation_but_lose_line_breaks() {
        assert_eq!(normalize_title("  Q3/Q4: review?  "), "Q3/Q4: review?");
        assert_eq!(normalize_title("one\n two\tthree"), "one two three");
        assert_eq!(normalize_title("bell\u{7}\u{202E}"), "bell");
        assert_eq!(normalize_title("...env"), "...env");
        assert_eq!(normalize_title(" \n\t "), "");
    }

    #[test]
    fn folder_names_are_date_time_and_title() {
        let start: StartTime = "2026-10-27T09:30:00+01:00".parse().unwrap();
        assert_eq!(folder_name(&start, "Standup"), "2026-10-27 0930 Standup");
        assert_eq!(
            folder_name(&start, DEFAULT_FOLDER_TITLE),
            "2026-10-27 0930 Meeting"
        );
    }
}
