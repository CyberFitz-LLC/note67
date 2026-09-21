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

/// How far apart two matched anchors may be placed and still be the same
/// alignment.
///
/// Two transcripts of one meeting disagree about where a phrase starts by a
/// second or two — different segmenters, different silence handling. Beyond
/// that they are describing different moments.
pub const CLUSTER_TOLERANCE_MS: i64 = 2_500;

/// The smallest cluster that can carry an alignment.
///
/// Three consistent anchors is a real signal; the previous rule wanted four
/// **in total**, which is a different question. A recording started late shares
/// only its overlap with the other, and a short overlap honestly contains few
/// anchors — refusing it for that reason rejects exactly the case this feature
/// exists for.
pub const MIN_CLUSTER: usize = 3;

/// What lining two transcripts up actually found.
#[derive(Debug, Clone, PartialEq)]
pub struct Alignment {
    /// How far `other` is ahead of `base`, in milliseconds.
    pub offset_ms: i64,
    /// Anchors that agree with this offset.
    pub agreeing: usize,
    /// Anchors that matched on text at all, including ones that disagree.
    ///
    /// The ratio between the two is the evidence: many matches that scatter
    /// across minutes are coincidence, a few that agree tightly are an
    /// alignment.
    pub matched: usize,
    /// How far apart the agreeing anchors sit.
    pub spread_ms: i64,
}

/// Line two transcripts up, and say what the evidence was.
///
/// `None` when no consistent alignment exists at all.
///
/// Judged by **consistency rather than count**. Two recordings of one meeting
/// produce anchors that all agree on a single offset; two unrelated recordings
/// produce coincidental matches on stock phrases that scatter. Counting matches
/// cannot tell those apart, and it fails the ordinary case of a meeting joined
/// late, where the overlap is short and the anchors in it are few but unanimous.
pub fn align(base: &[Anchor], other: &[Anchor]) -> Option<Alignment> {
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

    if deltas.is_empty() {
        return None;
    }
    deltas.sort_unstable();

    // The largest run of deltas that sit within tolerance of each other. A
    // single mismatched anchor — the same stock phrase said twice in an hour —
    // falls outside the run rather than dragging it, which a mean would not
    // survive.
    let (mut best_start, mut best_len) = (0usize, 0usize);
    for start in 0..deltas.len() {
        let mut end = start;
        while end + 1 < deltas.len() && deltas[end + 1] - deltas[start] <= CLUSTER_TOLERANCE_MS {
            end += 1;
        }
        if end - start + 1 > best_len {
            best_len = end - start + 1;
            best_start = start;
        }
    }

    if best_len < MIN_CLUSTER {
        return None;
    }

    let cluster = &deltas[best_start..best_start + best_len];
    Some(Alignment {
        offset_ms: cluster[cluster.len() / 2],
        agreeing: best_len,
        matched: deltas.len(),
        spread_ms: cluster[cluster.len() - 1] - cluster[0],
    })
}

/// How far `other` is ahead of `base`, in milliseconds.
///
/// `None` when the two do not look like the same meeting.
pub fn estimate_offset(base: &[Anchor], other: &[Anchor]) -> Option<i64> {
    align(base, other).map(|a| a.offset_ms)
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
    #[test]
    fn a_meeting_joined_late_aligns_on_its_overlap() {
        // The reported case: Note67 is started by hand, so it covers the last
        // part of a Teams recording. The overlap is real and consistent, and
        // the old rule refused it for having few anchors — which is what a
        // short overlap honestly has.
        let base = vec![
            anchor(0, "so the migration timeline is the thing"),
            anchor(30_000, "we need to decide on the cutover date"),
            anchor(60_000, "and who owns the rollback plan"),
        ];
        // The same three, twenty minutes into the other recording.
        let other = vec![
            anchor(1_200_000, "so the migration timeline is the thing"),
            anchor(1_230_000, "we need to decide on the cutover date"),
            anchor(1_260_100, "and who owns the rollback plan"),
        ];

        let a = align(&base, &other).expect("a late join is still the same meeting");
        assert!(
            (a.offset_ms - 1_200_000).abs() < CLUSTER_TOLERANCE_MS,
            "offset {} is not the twenty minutes it should be",
            a.offset_ms
        );
        assert_eq!(a.agreeing, 3);
        assert!(a.spread_ms <= CLUSTER_TOLERANCE_MS);
    }

    #[test]
    fn either_recording_may_be_the_longer_one() {
        // Manual start means it can go both ways, and the sign of the offset is
        // the only difference.
        let early = vec![
            anchor(0, "let us start with the budget"),
            anchor(20_000, "the second quarter numbers are in"),
            anchor(40_000, "and marketing came in under"),
        ];
        let late: Vec<Anchor> = early
            .iter()
            .map(|a| anchor(a.start_ms + 900_000, &a.text))
            .collect();

        let forward = align(&early, &late).expect("aligns");
        let backward = align(&late, &early).expect("aligns the other way");
        assert!((forward.offset_ms - 900_000).abs() < CLUSTER_TOLERANCE_MS);
        assert!((backward.offset_ms + 900_000).abs() < CLUSTER_TOLERANCE_MS);
    }

    #[test]
    fn coincidental_matches_that_scatter_are_still_refused() {
        // The safety property, and the reason this is judged by consistency
        // rather than by count. Two different meetings share stock phrases, and
        // those matches land minutes apart — merging on them would attribute
        // speech to people who were not there, plausibly.
        let base = vec![
            anchor(0, "yeah that makes sense to me"),
            anchor(60_000, "sorry could you say that again"),
            anchor(120_000, "yeah that makes sense to me"),
            anchor(180_000, "sorry could you say that again"),
            anchor(240_000, "yeah that makes sense to me"),
        ];
        let other = vec![
            anchor(500_000, "yeah that makes sense to me"),
            anchor(90_000, "sorry could you say that again"),
            anchor(1_700_000, "yeah that makes sense to me"),
            anchor(2_400_000, "sorry could you say that again"),
        ];

        assert!(
            align(&base, &other).is_none(),
            "scattered coincidences were accepted as an alignment"
        );
    }

    #[test]
    fn one_stray_match_does_not_move_a_good_alignment() {
        let base = vec![
            anchor(0, "the migration timeline is the thing"),
            anchor(30_000, "decide on the cutover date"),
            anchor(60_000, "who owns the rollback plan"),
            anchor(90_000, "yeah that makes sense to me"),
        ];
        let other = vec![
            anchor(600_000, "the migration timeline is the thing"),
            anchor(630_000, "decide on the cutover date"),
            anchor(660_000, "who owns the rollback plan"),
            // The same stock phrase, an hour away.
            anchor(4_000_000, "yeah that makes sense to me"),
        ];

        let a = align(&base, &other).expect("aligns");
        assert!(
            (a.offset_ms - 600_000).abs() < CLUSTER_TOLERANCE_MS,
            "a stray match dragged the offset to {}",
            a.offset_ms
        );
        assert_eq!(a.agreeing, 3);
        assert_eq!(a.matched, 4, "the stray should be counted, not hidden");
    }

    #[test]
    fn two_matching_moments_are_not_enough() {
        // Two points define a line through anything. Three is the smallest
        // number that can disagree.
        let base = vec![anchor(0, "the migration timeline"), anchor(30_000, "the cutover date")];
        let other = vec![
            anchor(600_000, "the migration timeline"),
            anchor(630_000, "the cutover date"),
        ];
        assert!(align(&base, &other).is_none());
    }

    #[test]
    fn nothing_in_common_aligns_to_nothing() {
        let base = vec![anchor(0, "entirely unrelated words here")];
        let other = vec![anchor(0, "nothing whatsoever in common")];
        assert!(align(&base, &other).is_none());
    }

}
