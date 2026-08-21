//! Lining up two recordings of the same meeting.
//!
//! Two tools recording the same hour do not agree about when it started. Teams
//! stamps from the meeting's start, Otter from when its recorder joined, Note67
//! from when someone pressed record — and nobody's clock is the same. So the
//! offset between two transcripts has to be recovered from what was *said*,
//! not from what the timestamps claim.

/// Reduce text to what two transcription systems might plausibly agree on.
///
/// Casing, punctuation and filler spacing are exactly where independent
/// transcribers differ while hearing the same words, so comparing raw text
/// would find almost no matches between tools that were both correct.
pub fn normalize(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|word| {
            word.chars()
                .filter(|c| c.is_alphanumeric())
                .flat_map(|c| c.to_lowercase())
                .collect::<String>()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

/// How alike two utterances are, from 0 to 1.
///
/// Token overlap rather than edit distance: transcribers disagree by dropping
/// or inserting whole words far more often than by misspelling them, and
/// character distance punishes a missing "the" about as hard as a wrong noun.
pub fn similarity(a: &str, b: &str) -> f32 {
    let (left, right) = (normalize(a), normalize(b));
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }

    let mut remaining = right.clone();
    let mut shared = 0usize;
    for word in &left {
        if let Some(pos) = remaining.iter().position(|w| w == word) {
            remaining.remove(pos);
            shared += 1;
        }
    }

    // Over the shorter side, so a short utterance fully contained in a longer
    // one still scores high — which is the common case when one tool splits a
    // sentence the other keeps whole.
    shared as f32 / left.len().min(right.len()) as f32
}

/// A segment reduced to what alignment needs.
#[derive(Debug, Clone, PartialEq)]
pub struct Anchor {
    pub start_ms: i64,
    pub text: String,
}

/// Below this, two utterances are not the same utterance.
///
/// Deliberately high. A wrong anchor drags the whole alignment, and there are
/// usually plenty of confident matches in an hour of speech — so it is far
/// better to use fewer, surer ones.
pub const MATCH_THRESHOLD: f32 = 0.6;

/// Fewer confident matches than this and the offset is a guess.
///
/// Two transcripts of *different* meetings will still produce a handful of
/// coincidental matches on common phrases. Refusing to align on a handful is
/// what stops the merge quietly stitching together two unrelated recordings.
pub const MIN_ANCHORS: usize = 4;

/// How far `other` is ahead of `base`, in milliseconds.
///
/// `None` when the two do not look like the same meeting.
pub fn estimate_offset(base: &[Anchor], other: &[Anchor]) -> Option<i64> {
    let mut deltas: Vec<i64> = Vec::new();

    for b in base {
        let best = other
            .iter()
            .map(|o| (similarity(&b.text, &o.text), o))
            .filter(|(score, _)| *score >= MATCH_THRESHOLD)
            .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

        if let Some((_, o)) = best {
            deltas.push(o.start_ms - b.start_ms);
        }
    }

    if deltas.len() < MIN_ANCHORS {
        return None;
    }

    // The median, not the mean. A single mismatched anchor — the same stock
    // phrase said twice in an hour — would drag a mean by minutes, and the
    // result would look plausible while being wrong throughout.
    deltas.sort_unstable();
    Some(deltas[deltas.len() / 2])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anchor(start_ms: i64, text: &str) -> Anchor {
        Anchor {
            start_ms,
            text: text.to_string(),
        }
    }

    #[test]
    fn normalization_ignores_what_transcribers_disagree_about() {
        // Casing and punctuation are exactly where two correct transcripts of
        // the same speech differ.
        assert_eq!(normalize("Hello, world!"), normalize("hello world"));
        assert_eq!(normalize("  spaced   out  "), vec!["spaced", "out"]);
    }

    #[test]
    fn identical_text_is_a_perfect_match() {
        assert_eq!(similarity("shall we start", "shall we start"), 1.0);
    }

    #[test]
    fn punctuation_and_case_do_not_reduce_a_match() {
        assert_eq!(similarity("Shall we start?", "shall we start"), 1.0);
    }

    #[test]
    fn unrelated_speech_does_not_match() {
        assert!(similarity("shall we start", "the budget is approved") < 0.2);
    }

    #[test]
    fn a_phrase_contained_in_a_longer_one_still_matches() {
        // One tool splits a sentence the other keeps whole. Scoring over the
        // shorter side is what keeps that a match.
        assert!(similarity("shall we start", "okay so shall we start then") >= MATCH_THRESHOLD);
    }

    #[test]
    fn empty_text_matches_nothing() {
        // Otherwise every silent or punctuation-only segment would anchor to
        // everything.
        assert_eq!(similarity("", "anything"), 0.0);
        assert_eq!(similarity("...", "anything"), 0.0);
    }

    #[test]
    fn a_constant_offset_is_recovered() {
        let base = vec![
            anchor(0, "morning everyone"),
            anchor(5_000, "shall we start"),
            anchor(10_000, "first item is the budget"),
            anchor(15_000, "any objections to that"),
            anchor(20_000, "then we move on"),
        ];
        // The same meeting, recorded by a tool that started 30 seconds earlier.
        let other: Vec<Anchor> = base
            .iter()
            .map(|a| anchor(a.start_ms + 30_000, &a.text))
            .collect();

        assert_eq!(estimate_offset(&base, &other), Some(30_000));
    }

    #[test]
    fn a_negative_offset_is_recovered() {
        let base = vec![
            anchor(30_000, "morning everyone"),
            anchor(35_000, "shall we start"),
            anchor(40_000, "first item is the budget"),
            anchor(45_000, "any objections to that"),
        ];
        let other: Vec<Anchor> = base
            .iter()
            .map(|a| anchor(a.start_ms - 30_000, &a.text))
            .collect();
        assert_eq!(estimate_offset(&base, &other), Some(-30_000));
    }

    #[test]
    fn one_bad_anchor_does_not_drag_the_alignment() {
        // The median exists for this. A stock phrase repeated an hour later
        // would move a mean by minutes and the result would look plausible.
        let base = vec![
            anchor(0, "morning everyone"),
            anchor(5_000, "shall we start"),
            anchor(10_000, "first item is the budget"),
            anchor(15_000, "any objections to that"),
            anchor(20_000, "thanks everyone"),
        ];
        let mut other: Vec<Anchor> = base
            .iter()
            .map(|a| anchor(a.start_ms + 2_000, &a.text))
            .collect();
        // The same words said again much later.
        other.push(anchor(3_600_000, "thanks everyone"));

        assert_eq!(estimate_offset(&base, &other), Some(2_000));
    }

    #[test]
    fn two_different_meetings_do_not_align() {
        // The check that stops a merge stitching together unrelated
        // recordings. A few coincidental matches on common phrases are not
        // evidence of the same meeting.
        let base = vec![
            anchor(0, "morning everyone"),
            anchor(5_000, "the budget needs approval"),
            anchor(10_000, "we ship on friday"),
        ];
        let other = vec![
            anchor(0, "completely different discussion"),
            anchor(5_000, "about unrelated matters entirely"),
            anchor(10_000, "with nothing whatsoever in common"),
        ];
        assert_eq!(estimate_offset(&base, &other), None);
    }

    #[test]
    fn too_few_matches_is_refused_rather_than_guessed() {
        let base = vec![anchor(0, "morning everyone"), anchor(5_000, "shall we start")];
        let other = vec![anchor(1_000, "morning everyone")];
        assert_eq!(
            estimate_offset(&base, &other),
            None,
            "one anchor is not an alignment"
        );
    }

    #[test]
    fn an_empty_transcript_aligns_with_nothing() {
        assert_eq!(estimate_offset(&[], &[]), None);
    }
}
