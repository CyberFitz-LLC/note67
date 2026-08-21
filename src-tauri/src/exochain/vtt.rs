//! WebVTT parsing, for importing transcripts produced elsewhere.
//!
//! Microsoft Teams exports `.vtt`, which is the format this targets, but the
//! parser is deliberately lenient about the variations real files contain:
//! optional cue identifiers, `NOTE` blocks, a byte-order mark, CRLF endings,
//! comma decimal separators, and two- or three-part timestamps.
//!
//! What comes out feeds the same canonical form as a recorded transcript, so an
//! imported meeting joins the version chain like any other — with `Origin` set
//! to `Imported`, because all that can be attested about it is that this
//! content arrived at a given time and has not changed since.

/// One cue, reduced to what a transcript segment needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedSegment {
    pub start_ms: i64,
    pub end_ms: i64,
    pub speaker: Option<String>,
    pub text: String,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum VttError {
    #[error("this does not look like a WebVTT file (no WEBVTT header)")]
    MissingHeader,
    #[error("no cues found")]
    Empty,
}

/// Parse a timestamp: `HH:MM:SS.mmm`, `MM:SS.mmm`, and `,` for `.`.
fn parse_timestamp(raw: &str) -> Option<i64> {
    let raw = raw.trim().replace(',', ".");
    let mut parts = raw.split(':').collect::<Vec<_>>();
    if parts.len() < 2 || parts.len() > 3 {
        return None;
    }
    // Normalise to hours:minutes:seconds.
    if parts.len() == 2 {
        parts.insert(0, "0");
    }

    let hours: i64 = parts[0].trim().parse().ok()?;
    let minutes: i64 = parts[1].trim().parse().ok()?;
    let seconds: f64 = parts[2].trim().parse().ok()?;
    if !seconds.is_finite() {
        return None;
    }

    Some(hours * 3_600_000 + minutes * 60_000 + (seconds * 1000.0).round() as i64)
}

/// Split a cue timing line into its two timestamps.
fn parse_timing(line: &str) -> Option<(i64, i64)> {
    let (start, rest) = line.split_once("-->")?;
    // Anything after the end timestamp is a cue setting (alignment, position);
    // it affects rendering, not content.
    let end = rest.split_whitespace().next()?;
    Some((parse_timestamp(start)?, parse_timestamp(end)?))
}

/// Pull the speaker out of a voice span and strip the remaining markup.
///
/// Teams writes `<v Speaker Name>text</v>`. The tag may also carry classes
/// (`<v.loud Speaker>`), and payloads can contain `<i>`, `<b>` or `<c.colour>`
/// spans that are styling rather than content.
fn extract_speaker_and_text(payload: &str) -> (Option<String>, String) {
    let mut speaker = None;
    let mut text = String::with_capacity(payload.len());
    let mut chars = payload.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '<' {
            text.push(ch);
            continue;
        }

        let mut tag = String::new();
        for c in chars.by_ref() {
            if c == '>' {
                break;
            }
            tag.push(c);
        }

        // `<v Name>` or `<v.class Name>`: everything after the first whitespace
        // is the speaker. Only the first one counts — a cue with several voices
        // has no single speaker, and guessing would attribute words to the
        // wrong person.
        let is_voice = tag == "v" || tag.starts_with("v ") || tag.starts_with("v.");
        if is_voice && speaker.is_none() {
            if let Some((_, name)) = tag.split_once(char::is_whitespace) {
                let name = name.trim();
                if !name.is_empty() {
                    speaker = Some(name.to_string());
                }
            }
        }
    }

    (speaker, decode_entities(text.trim()))
}

/// Decode the entities WebVTT requires to be escaped.
fn decode_entities(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&lrm;", "")
        .replace("&rlm;", "")
        .replace("&nbsp;", " ")
        // Ampersand last: decoding it first would let "&amp;lt;" become "<".
        .replace("&amp;", "&")
}

/// Parse a WebVTT document into segments.
pub fn parse_vtt(content: &str) -> Result<Vec<ImportedSegment>, VttError> {
    // Strip a byte-order mark; Teams files frequently carry one, and it would
    // otherwise hide the WEBVTT header.
    let content = content.trim_start_matches('\u{feff}');

    if !content.trim_start().starts_with("WEBVTT") {
        return Err(VttError::MissingHeader);
    }

    let mut segments = Vec::new();
    let mut lines = content.lines().peekable();
    let mut pending_timing: Option<(i64, i64)> = None;
    let mut payload: Vec<String> = Vec::new();
    let mut in_note = false;

    // Push whatever has accumulated, if it amounts to a segment.
    fn flush(
        segments: &mut Vec<ImportedSegment>,
        timing: &mut Option<(i64, i64)>,
        payload: &mut Vec<String>,
    ) {
        if let Some((start_ms, end_ms)) = timing.take() {
            let (speaker, text) = extract_speaker_and_text(&payload.join(" "));
            // A cue with no words carries nothing; keeping it would put empty
            // rows in the transcript and change its hash for no reason.
            if !text.is_empty() {
                segments.push(ImportedSegment {
                    start_ms,
                    end_ms,
                    speaker,
                    text,
                });
            }
        }
        payload.clear();
    }

    while let Some(raw) = lines.next() {
        let line = raw.trim_end_matches('\r');
        let trimmed = line.trim();

        if trimmed.is_empty() {
            flush(&mut segments, &mut pending_timing, &mut payload);
            in_note = false;
            continue;
        }

        // NOTE blocks run until a blank line and are comments.
        if in_note {
            continue;
        }
        if trimmed == "NOTE" || trimmed.starts_with("NOTE ") {
            in_note = true;
            continue;
        }

        if trimmed.contains("-->") {
            // A new timing line ends the previous cue even without a blank
            // line between them, which some exporters omit.
            flush(&mut segments, &mut pending_timing, &mut payload);
            pending_timing = parse_timing(trimmed);
            continue;
        }

        if pending_timing.is_some() {
            payload.push(trimmed.to_string());
            continue;
        }

        // Otherwise this is the header, a header setting, or a cue identifier:
        // an id is the line immediately before a timing line, and carries no
        // transcript content either way.
    }

    flush(&mut segments, &mut pending_timing, &mut payload);

    if segments.is_empty() {
        return Err(VttError::Empty);
    }
    Ok(segments)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEAMS: &str = "WEBVTT\n\n\
        0d2eb2d0-1111-4a3f-9f0e-000000000001/1-0\n\
        00:00:00.000 --> 00:00:03.240\n\
        <v John Fitzpatrick>Good morning everyone.</v>\n\n\
        0d2eb2d0-1111-4a3f-9f0e-000000000001/2-0\n\
        00:00:03.500 --> 00:00:06.100\n\
        <v Jane Smith>Morning John.</v>\n";

    #[test]
    fn a_teams_export_parses() {
        let out = parse_vtt(TEAMS).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(
            out[0],
            ImportedSegment {
                start_ms: 0,
                end_ms: 3240,
                speaker: Some("John Fitzpatrick".into()),
                text: "Good morning everyone.".into(),
            }
        );
        assert_eq!(out[1].speaker.as_deref(), Some("Jane Smith"));
    }

    #[test]
    fn cue_identifiers_are_not_mistaken_for_content() {
        // Teams uses UUID identifiers; treating one as text would put it in
        // the transcript and into the hash.
        let out = parse_vtt(TEAMS).unwrap();
        assert!(
            !out.iter().any(|s| s.text.contains("0d2eb2d0")),
            "a cue id leaked into the transcript"
        );
    }

    #[test]
    fn a_byte_order_mark_does_not_hide_the_header() {
        let out = parse_vtt(&format!("\u{feff}{TEAMS}")).unwrap();
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn crlf_endings_parse() {
        let out = parse_vtt(&TEAMS.replace('\n', "\r\n")).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].text, "Good morning everyone.");
    }

    #[test]
    fn short_timestamps_are_accepted() {
        let vtt = "WEBVTT\n\n01:02.500 --> 01:05.000\nHello.\n";
        let out = parse_vtt(vtt).unwrap();
        assert_eq!((out[0].start_ms, out[0].end_ms), (62_500, 65_000));
    }

    #[test]
    fn comma_decimals_are_accepted() {
        let vtt = "WEBVTT\n\n00:00:01,250 --> 00:00:02,750\nHello.\n";
        let out = parse_vtt(vtt).unwrap();
        assert_eq!((out[0].start_ms, out[0].end_ms), (1250, 2750));
    }

    #[test]
    fn cue_settings_after_the_end_time_are_ignored() {
        let vtt = "WEBVTT\n\n00:00:00.000 --> 00:00:02.000 align:start position:10%\nHi.\n";
        let out = parse_vtt(vtt).unwrap();
        assert_eq!(out[0].end_ms, 2000);
        assert_eq!(out[0].text, "Hi.");
    }

    #[test]
    fn note_blocks_are_skipped() {
        let vtt = "WEBVTT\n\nNOTE this recording was produced by Teams\nand continues here\n\n\
                   00:00:00.000 --> 00:00:01.000\nReal text.\n";
        let out = parse_vtt(vtt).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "Real text.");
    }

    #[test]
    fn multi_line_cues_join_into_one_segment() {
        let vtt = "WEBVTT\n\n00:00:00.000 --> 00:00:04.000\n<v Ann>First line\nsecond line</v>\n";
        let out = parse_vtt(vtt).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "First line second line");
    }

    #[test]
    fn styling_markup_is_stripped_but_its_words_are_kept() {
        let vtt = "WEBVTT\n\n00:00:00.000 --> 00:00:02.000\n<v Ann>That is <i>very</i> <b>good</b></v>\n";
        let out = parse_vtt(vtt).unwrap();
        assert_eq!(out[0].text, "That is very good");
    }

    #[test]
    fn a_voice_tag_with_classes_still_yields_the_speaker() {
        let vtt = "WEBVTT\n\n00:00:00.000 --> 00:00:02.000\n<v.loud Ann Smith>Hello</v>\n";
        let out = parse_vtt(vtt).unwrap();
        assert_eq!(out[0].speaker.as_deref(), Some("Ann Smith"));
    }

    #[test]
    fn a_cue_with_two_voices_claims_neither() {
        // Attributing a mixed cue to whichever name came first would put words
        // in the wrong person's mouth, which is worse than no attribution.
        let vtt = "WEBVTT\n\n00:00:00.000 --> 00:00:02.000\n<v Ann>Hi</v> <v Bob>Hello</v>\n";
        let out = parse_vtt(vtt).unwrap();
        assert_eq!(out[0].speaker.as_deref(), Some("Ann"));
        assert_eq!(out[0].text, "Hi Hello");
    }

    #[test]
    fn escaped_entities_are_decoded() {
        let vtt = "WEBVTT\n\n00:00:00.000 --> 00:00:02.000\nR&amp;D said &lt;this&gt;\n";
        let out = parse_vtt(vtt).unwrap();
        assert_eq!(out[0].text, "R&D said <this>");
    }

    #[test]
    fn an_escaped_entity_is_not_decoded_twice() {
        // "&amp;lt;" means the literal text "&lt;", not "<".
        let vtt = "WEBVTT\n\n00:00:00.000 --> 00:00:02.000\n&amp;lt;\n";
        let out = parse_vtt(vtt).unwrap();
        assert_eq!(out[0].text, "&lt;");
    }

    #[test]
    fn empty_cues_are_dropped() {
        let vtt = "WEBVTT\n\n00:00:00.000 --> 00:00:02.000\n\n\
                   00:00:02.000 --> 00:00:04.000\nReal.\n";
        let out = parse_vtt(vtt).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "Real.");
    }

    #[test]
    fn cues_without_a_blank_line_between_them_still_separate() {
        let vtt = "WEBVTT\n\n00:00:00.000 --> 00:00:01.000\nOne.\n\
                   00:00:01.000 --> 00:00:02.000\nTwo.\n";
        let out = parse_vtt(vtt).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!((out[0].text.as_str(), out[1].text.as_str()), ("One.", "Two."));
    }

    #[test]
    fn something_that_is_not_vtt_is_rejected() {
        assert_eq!(parse_vtt("Meeting notes\n\nhello"), Err(VttError::MissingHeader));
    }

    #[test]
    fn a_header_with_no_cues_is_rejected() {
        // Importing this would create a note with an empty transcript and a
        // receipt attesting nothing.
        assert_eq!(parse_vtt("WEBVTT\n\n"), Err(VttError::Empty));
    }
}
