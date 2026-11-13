//! Cleans the text of one Whisper segment.
//!
//! Whisper writes tags instead of words for sound that is not speech:
//! `[BLANK_AUDIO]`, `[ Silence ]`, `(upbeat music)`, `[MUSIC PLAYING]`. They
//! are not part of what anyone said, so they never reach the transcript.

/// Longest tag content treated as a non-speech marker. Longer bracketed text
/// is more likely to be something that was said.
const MAX_MARKER_CHARS: usize = 40;

/// Returns the text of a segment ready for the transcript, or `None` when the
/// segment holds no speech.
///
/// - Whitespace is trimmed and runs of whitespace become one space.
/// - A segment made only of markers, punctuation or symbols is dropped.
/// - Square-bracketed markers are also removed from a segment that has words
///   around them. Parenthesised text next to words is kept, because it can be
///   real content.
///
/// A marker is `[...]` or `(...)` whose content is short and made of letters,
/// spaces, `_`, `-` and `'` only. Digits disqualify it, so `(555)` and `[1]`
/// are left alone.
pub(crate) fn clean_segment_text(raw: &str) -> Option<String> {
    let chars: Vec<char> = raw.chars().collect();
    // The text with every marker removed, used to decide whether any speech
    // is left, and the text with only square-bracketed markers removed, which
    // is what is kept.
    let mut without_markers = String::with_capacity(raw.len());
    let mut kept = String::with_capacity(raw.len());

    let mut i = 0;
    while i < chars.len() {
        let open = chars[i];
        if let Some(close) = closing_bracket(open)
            && let Some(len) = marker_len(&chars[i..], open, close)
        {
            if open == '(' {
                kept.extend(&chars[i..i + len]);
            } else {
                kept.push(' ');
            }
            without_markers.push(' ');
            i += len;
            continue;
        }
        without_markers.push(open);
        kept.push(open);
        i += 1;
    }

    if !without_markers.chars().any(char::is_alphanumeric) {
        return None;
    }
    let cleaned = kept.split_whitespace().collect::<Vec<_>>().join(" ");
    (!cleaned.is_empty()).then_some(cleaned)
}

fn closing_bracket(open: char) -> Option<char> {
    match open {
        '[' => Some(']'),
        '(' => Some(')'),
        _ => None,
    }
}

/// If `chars` starts with a marker opened by `open`, returns how many chars
/// the marker spans, brackets included.
fn marker_len(chars: &[char], open: char, close: char) -> Option<usize> {
    let end = chars
        .iter()
        .skip(1)
        .take(MAX_MARKER_CHARS + 1)
        .position(|&c| c == close || c == open)?
        + 1;
    if chars[end] != close {
        return None;
    }
    let content = &chars[1..end];
    let allowed =
        |c: &char| c.is_alphabetic() || c.is_whitespace() || matches!(c, '_' | '-' | '\'');
    let is_marker = content.iter().all(allowed) && content.iter().any(|c| c.is_alphabetic());
    is_marker.then_some(end + 1)
}

#[cfg(test)]
mod tests {
    use super::clean_segment_text;

    /// Asserts that `raw` cleans to `expected`, where `None` means dropped.
    #[track_caller]
    fn check(raw: &str, expected: Option<&str>) {
        assert_eq!(
            clean_segment_text(raw).as_deref(),
            expected,
            "cleaning {raw:?}"
        );
    }

    #[test]
    fn trims_and_collapses_whitespace() {
        check(" Hello there.", Some("Hello there."));
        check("\tHello   there.\n", Some("Hello there."));
        check("Hello\nthere.", Some("Hello there."));
    }

    #[test]
    fn drops_empty_segments() {
        for raw in ["", " ", "\n\t  "] {
            check(raw, None);
        }
    }

    #[test]
    fn drops_bracketed_markers() {
        for raw in [
            "[BLANK_AUDIO]",
            " [BLANK_AUDIO]",
            "[ Silence ]",
            "[MUSIC PLAYING]",
            "[inaudible]",
            "[no audio]",
            "[Musik]",
            "[música]",
        ] {
            check(raw, None);
        }
    }

    #[test]
    fn drops_parenthesised_markers() {
        for raw in [
            "(music)",
            " (upbeat music)",
            "(audience laughing)",
            "(speaking in foreign language)",
            "(door closes)",
            "(Laughter)",
        ] {
            check(raw, None);
        }
    }

    #[test]
    fn drops_several_markers_in_one_segment() {
        check("[BLANK_AUDIO] [BLANK_AUDIO]", None);
        check("[MUSIC] (applause)", None);
        check("(music) - [ Silence ].", None);
    }

    #[test]
    fn drops_segments_with_no_words() {
        for raw in ["...", "-", "♪", "♪♪♪", " . ", "[BLANK_AUDIO].", "*"] {
            check(raw, None);
        }
    }

    #[test]
    fn strips_square_markers_around_speech() {
        check("[BLANK_AUDIO] Okay.", Some("Okay."));
        check("Okay. [MUSIC PLAYING]", Some("Okay."));
        check("Hello [inaudible] there.", Some("Hello there."));
        check("[MUSIC] Hello. [ Silence ]", Some("Hello."));
    }

    #[test]
    fn keeps_parentheses_next_to_speech() {
        check("That works (I think).", Some("That works (I think)."));
        check("(laughs) Sure.", Some("(laughs) Sure."));
    }

    #[test]
    fn keeps_brackets_that_are_not_markers() {
        // Digits, punctuation inside, unbalanced or over-long content.
        check("Call (555) 0100.", Some("Call (555) 0100."));
        check("(555)", Some("(555)"));
        check("See [1] and [2].", Some("See [1] and [2]."));
        check("[BLANK_AUDIO", Some("[BLANK_AUDIO"));
        check("BLANK_AUDIO]", Some("BLANK_AUDIO]"));
        check("[what? no]", Some("[what? no]"));
        let long = "(this is a full sentence that somebody really did say out loud)";
        check(long, Some(long));
    }

    #[test]
    fn keeps_plain_speech_untouched() {
        check(
            " The quick brown fox jumps over the lazy dog.",
            Some("The quick brown fox jumps over the lazy dog."),
        );
        check("Yes.", Some("Yes."));
        check("42", Some("42"));
    }

    #[test]
    fn keeps_speech_in_other_scripts() {
        check(" こんにちは。", Some("こんにちは。"));
        check("Привет.", Some("Привет."));
        check("[音楽]", None);
    }

    #[test]
    fn empty_brackets_are_not_markers() {
        check("[]", None); // no words either way
        check("[ ] ok", Some("[ ] ok"));
    }
}
