//! Combining several recordings of one meeting.
//!
//! Note67, Teams and Otter all recording the same hour is not three notes. It
//! is one meeting seen three ways, and **where they disagree is information**.
//! Teams knows who was speaking because it has each participant on a separate
//! stream; Note67 has the room. Aligning them produces attribution that neither
//! could reach alone.
//!
//! This module decides what the merged transcript says. It does not touch the
//! database, so the decisions can be tested without one.

pub mod align;

use align::{estimate_offset, similarity, Anchor};

/// A segment as it arrives from some source.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceSegment {
    pub start_ms: i64,
    pub end_ms: i64,
    /// `None` when the source does not attribute speech at all.
    pub speaker: Option<String>,
    pub text: String,
}

/// Labels that name a track rather than a person.
///
/// Note67 attributes by which file a segment came from, so everything on the
/// microphone is "You" and everything else is "Others". Those carry real
/// meaning — "You" is genuinely you — but they are not names, and a merge
/// exists to replace them with names.
const GENERIC_LABELS: [&str; 4] = ["you", "others", "speaker", "unknown"];

pub fn is_generic(label: &str) -> bool {
    let lowered = label.trim().to_lowercase();
    GENERIC_LABELS.contains(&lowered.as_str())
        // "Speaker 1", "Speaker 2" — a diarizer's placeholder, not a name.
        || lowered
            .strip_prefix("speaker")
            .is_some_and(|rest| rest.trim().chars().all(|c| c.is_ascii_digit()))
}

/// How much two spans overlap, in milliseconds.
pub fn overlap_ms(a: (i64, i64), b: (i64, i64)) -> i64 {
    (a.1.min(b.1) - a.0.max(b.0)).max(0)
}

/// A merged segment and what is known about where it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct MergedSegment {
    pub start_ms: i64,
    pub end_ms: i64,
    pub speaker: Option<String>,
    pub text: String,
    /// Which source supplied the speaker, when one did. Recorded because a name
    /// taken from another tool is a weaker claim than one this app observed,
    /// and a receipt over the result must not blur the two.
    pub speaker_source: Option<String>,
    /// Set when a source's text for this span differs materially. A
    /// disagreement is worth keeping rather than resolving silently — it is
    /// the honest signal that one of the two is wrong.
    pub disagreement: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MergeReport {
    pub offset_ms: Option<i64>,
    pub segments_named: usize,
    pub disagreements: usize,
    /// True when the two do not look like the same meeting.
    pub rejected: bool,
    /// What lining the two up actually found, whether or not it was enough.
    ///
    /// Reported on a refusal as well as a merge. "That does not look like the
    /// same meeting" is a conclusion; this is the evidence for it, and without
    /// it a user has no way to tell a genuinely different recording from one
    /// that overlapped too little to be sure.
    pub evidence: MatchEvidence,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MatchEvidence {
    /// Segments whose text matched something in the other transcript.
    pub matched: usize,
    /// Of those, how many agreed on one alignment.
    pub agreeing: usize,
    /// Segments in each transcript, so a short overlap can be seen for what it
    /// is rather than mistaken for a poor match.
    pub base_segments: usize,
    pub other_segments: usize,
    /// The stretch of the base transcript the other one covers, once aligned.
    pub overlap_ms: i64,
}

/// Below this share of a base segment's duration, an overlap is incidental.
///
/// Speaker turns butt up against each other, so a segment will always clip its
/// neighbour by a few milliseconds. Taking a name from that would attribute
/// speech to whoever spoke just before.
pub const MIN_OVERLAP_RATIO: f32 = 0.5;

/// Text agreeing at least this much is the same utterance said two ways.
///
/// Two transcripts of the same speech share most of their words, so half
/// agreement is not a transcription difference — it means one of them heard
/// something materially different, which is exactly what is worth surfacing.
pub const SAME_UTTERANCE: f32 = 0.6;

/// Enrich a base transcript with what another recording of the same meeting knows.
///
/// The base keeps its own text and timings — it is the recording this app made,
/// and its audio is the one the receipts are about. What it takes from the
/// other source is **attribution**, which is the thing it cannot produce
/// itself.
pub fn merge_speakers(
    base: &[SourceSegment],
    other: &[SourceSegment],
    source_name: &str,
) -> (Vec<MergedSegment>, MergeReport) {
    let base_anchors: Vec<Anchor> = base
        .iter()
        .map(|s| Anchor {
            start_ms: s.start_ms,
            text: s.text.clone(),
        })
        .collect();
    let other_anchors: Vec<Anchor> = other
        .iter()
        .map(|s| Anchor {
            start_ms: s.start_ms,
            text: s.text.clone(),
        })
        .collect();

    let alignment = crate::merge::align::align(&base_anchors, &other_anchors);
    let mut evidence = MatchEvidence {
        base_segments: base.len(),
        other_segments: other.len(),
        matched: alignment.as_ref().map(|a| a.matched).unwrap_or(0),
        agreeing: alignment.as_ref().map(|a| a.agreeing).unwrap_or(0),
        overlap_ms: 0,
    };

    let Some(offset) = alignment.map(|a| a.offset_ms) else {
        // Refused rather than merged at zero offset. Merging two unrelated
        // recordings would attribute speech to people who were not there, and
        // the result would look entirely plausible.
        return (
            base.iter().map(passthrough).collect(),
            MergeReport {
                rejected: true,
                evidence,
                ..Default::default()
            },
        );
    };

    // How much of the base the other actually covers, once aligned. A short
    // overlap is the normal shape when a recording is started by hand, and
    // saying so is the difference between "these do not match" and "these share
    // twelve minutes".
    let shifted: Vec<(i64, i64)> = other
        .iter()
        .map(|s| (s.start_ms - offset, s.end_ms - offset))
        .collect();
    evidence.overlap_ms = base
        .iter()
        .map(|b| {
            shifted
                .iter()
                .map(|o| overlap_ms((b.start_ms, b.end_ms), *o))
                .sum::<i64>()
        })
        .sum();

    let mut report = MergeReport {
        offset_ms: Some(offset),
        evidence,
        ..Default::default()
    };
    let mut merged = Vec::with_capacity(base.len());

    for segment in base {
        let mut out = passthrough(segment);
        let span = (segment.start_ms, segment.end_ms);
        let duration = (segment.end_ms - segment.start_ms).max(1);

        // The other source's segment sharing the most time with this one, once
        // its clock is shifted onto ours.
        let best = other
            .iter()
            .map(|o| {
                let shifted = (o.start_ms - offset, o.end_ms - offset);
                (overlap_ms(span, shifted), o)
            })
            .filter(|(ms, _)| *ms as f32 / duration as f32 >= MIN_OVERLAP_RATIO)
            .max_by_key(|(ms, _)| *ms);

        if let Some((_, counterpart)) = best {
            if let Some(name) = counterpart.speaker.as_deref().filter(|n| !is_generic(n)) {
                // A name never loses to a track label, and never overwrites
                // another name: two sources disagreeing about who spoke is a
                // disagreement to record, not a race to settle by ordering.
                match out.speaker.as_deref() {
                    Some(existing) if !is_generic(existing) => {
                        if existing != name {
                            out.disagreement =
                                Some(format!("{source_name} attributes this to {name}"));
                            report.disagreements += 1;
                        }
                    }
                    _ => {
                        out.speaker = Some(name.to_string());
                        out.speaker_source = Some(source_name.to_string());
                        report.segments_named += 1;
                    }
                }
            }

            if similarity(&segment.text, &counterpart.text) < SAME_UTTERANCE {
                // Overlapping in time but not in words. Usually crosstalk one
                // tool caught and the other missed — worth surfacing, never
                // worth silently replacing our own audio's text with.
                out.disagreement = Some(match out.disagreement {
                    Some(existing) => format!("{existing}; {source_name}: \"{}\"", counterpart.text),
                    None => format!("{source_name}: \"{}\"", counterpart.text),
                });
                report.disagreements += 1;
            }
        }

        merged.push(out);
    }

    (merged, report)
}

fn passthrough(segment: &SourceSegment) -> MergedSegment {
    MergedSegment {
        start_ms: segment.start_ms,
        end_ms: segment.end_ms,
        speaker: segment.speaker.clone(),
        text: segment.text.clone(),
        speaker_source: None,
        disagreement: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(start_ms: i64, end_ms: i64, speaker: Option<&str>, text: &str) -> SourceSegment {
        SourceSegment {
            start_ms,
            end_ms,
            speaker: speaker.map(str::to_string),
            text: text.to_string(),
        }
    }

    /// A Note67 recording: attribution by track, so "You" and "Others".
    fn note67() -> Vec<SourceSegment> {
        vec![
            seg(0, 3_000, Some("You"), "morning everyone shall we start"),
            seg(3_000, 7_000, Some("Others"), "yes lets begin with the budget"),
            seg(7_000, 11_000, Some("Others"), "i have concerns about the timeline"),
            seg(11_000, 15_000, Some("You"), "noted lets take that offline"),
            seg(15_000, 19_000, Some("Others"), "agreed we can move on"),
        ]
    }

    /// The same meeting from Teams, which knows the names, and whose clock
    /// started ten seconds earlier.
    fn teams() -> Vec<SourceSegment> {
        vec![
            seg(10_000, 13_000, Some("John Fitzpatrick"), "morning everyone shall we start"),
            seg(13_000, 17_000, Some("Bob Smith"), "yes lets begin with the budget"),
            seg(17_000, 21_000, Some("Walley Chen"), "i have concerns about the timeline"),
            seg(21_000, 25_000, Some("John Fitzpatrick"), "noted lets take that offline"),
            seg(25_000, 29_000, Some("Bob Smith"), "agreed we can move on"),
        ]
    }

    #[test]
    fn track_labels_are_recognised_as_not_being_names() {
        for label in ["You", "others", "Speaker", "Speaker 1", "SPEAKER 12", "unknown"] {
            assert!(is_generic(label), "{label} should be generic");
        }
        for name in ["Bob Smith", "Walley", "John Fitzpatrick"] {
            assert!(!is_generic(name), "{name} should be a name");
        }
    }

    #[test]
    fn overlapping_spans_are_measured_and_disjoint_ones_are_not() {
        assert_eq!(overlap_ms((0, 1000), (500, 1500)), 500);
        assert_eq!(overlap_ms((0, 1000), (1000, 2000)), 0);
        assert_eq!(overlap_ms((0, 1000), (2000, 3000)), 0);
    }

    #[test]
    fn teams_names_replace_note67s_track_labels() {
        // The whole point. Note67 knows "someone else spoke"; Teams knows who.
        let (merged, report) = merge_speakers(&note67(), &teams(), "Teams");

        assert_eq!(report.offset_ms, Some(10_000));
        assert!(!report.rejected);
        assert_eq!(report.segments_named, 5);

        let names: Vec<Option<&str>> = merged.iter().map(|m| m.speaker.as_deref()).collect();
        assert_eq!(
            names,
            vec![
                Some("John Fitzpatrick"),
                Some("Bob Smith"),
                Some("Walley Chen"),
                Some("John Fitzpatrick"),
                Some("Bob Smith"),
            ]
        );
    }

    #[test]
    fn the_base_keeps_its_own_text_and_timings() {
        // Note67's audio is what the receipts are about. It takes attribution
        // from the other source, not content.
        let (merged, _) = merge_speakers(&note67(), &teams(), "Teams");
        for (m, b) in merged.iter().zip(note67()) {
            assert_eq!(m.text, b.text);
            assert_eq!(m.start_ms, b.start_ms);
            assert_eq!(m.end_ms, b.end_ms);
        }
    }

    #[test]
    fn a_borrowed_name_records_where_it_came_from() {
        // A name from another tool is a weaker claim than one this app
        // observed, and a receipt must not blur the two.
        let (merged, _) = merge_speakers(&note67(), &teams(), "Teams");
        assert_eq!(merged[0].speaker_source.as_deref(), Some("Teams"));
    }

    #[test]
    fn merging_twice_changes_nothing() {
        // Importing the same export again is an ordinary mistake, and it must
        // not double anything or shuffle attribution.
        let (once, _) = merge_speakers(&note67(), &teams(), "Teams");
        let as_source: Vec<SourceSegment> = once
            .iter()
            .map(|m| seg(m.start_ms, m.end_ms, m.speaker.as_deref(), &m.text))
            .collect();
        let (twice, report) = merge_speakers(&as_source, &teams(), "Teams");

        // Compared on what the transcript says rather than on the whole
        // struct: `speaker_source` legitimately differs, because the second
        // pass took no name from anywhere — the names were already there.
        let said = |v: &[MergedSegment]| -> Vec<(i64, Option<String>, String)> {
            v.iter()
                .map(|m| (m.start_ms, m.speaker.clone(), m.text.clone()))
                .collect()
        };
        assert_eq!(said(&once), said(&twice));
        assert_eq!(report.segments_named, 0, "nothing left to name");
        assert_eq!(report.disagreements, 0, "a second pass invented a conflict");
    }

    #[test]
    fn a_different_meeting_is_refused_rather_than_merged() {
        // Merging unrelated recordings would attribute speech to people who
        // were not there, and the result would look entirely plausible.
        let unrelated = vec![
            seg(0, 3_000, Some("Someone Else"), "completely different discussion"),
            seg(3_000, 6_000, Some("Another Person"), "about unrelated matters"),
            seg(6_000, 9_000, Some("Someone Else"), "nothing whatsoever in common"),
        ];
        let (merged, report) = merge_speakers(&note67(), &unrelated, "Otter");

        assert!(report.rejected);
        assert_eq!(report.segments_named, 0);
        assert_eq!(
            merged[0].speaker.as_deref(),
            Some("You"),
            "the base was left exactly as it was"
        );
    }

    #[test]
    fn two_sources_disagreeing_about_a_speaker_is_recorded_not_resolved() {
        // Ordering should not decide who spoke. Keeping the disagreement is
        // the honest outcome; one of the two is wrong and the user can see it.
        let named: Vec<SourceSegment> = note67()
            .iter()
            .enumerate()
            .map(|(i, s)| {
                seg(
                    s.start_ms,
                    s.end_ms,
                    Some(if i == 1 { "Christian" } else { "John Fitzpatrick" }),
                    &s.text,
                )
            })
            .collect();

        let (merged, report) = merge_speakers(&named, &teams(), "Teams");
        assert!(merged[1].disagreement.as_deref().unwrap().contains("Bob Smith"));
        assert_eq!(
            merged[1].speaker.as_deref(),
            Some("Christian"),
            "the existing name was not overwritten"
        );
        assert!(report.disagreements >= 1);
    }

    #[test]
    fn text_the_other_source_heard_differently_is_surfaced() {
        // Usually crosstalk one tool caught and the other missed. Worth
        // showing, never worth silently replacing our own audio's text with.
        let mut other = teams();
        other[2] = seg(
            17_000,
            21_000,
            Some("Walley Chen"),
            "actually i think the timeline is fine",
        );

        let (merged, report) = merge_speakers(&note67(), &other, "Teams");
        assert!(
            merged[2]
                .disagreement
                .as_deref()
                .unwrap()
                .contains("the timeline is fine"),
            "{:?}",
            merged[2].disagreement
        );
        assert_eq!(
            merged[2].text, "i have concerns about the timeline",
            "our own text stands"
        );
        assert!(report.disagreements >= 1);
    }

    #[test]
    fn a_glancing_overlap_does_not_borrow_a_name() {
        // Speaker turns butt against each other. Taking a name from a few
        // milliseconds of contact would attribute speech to whoever spoke just
        // before.
        let base = note67();
        let mut other = teams();
        // Shift one Teams segment so it barely clips the second base segment.
        other[1] = seg(12_800, 13_200, Some("Bob Smith"), "yes lets begin with the budget");

        let (merged, _) = merge_speakers(&base, &other, "Teams");
        assert_ne!(
            merged[1].speaker_source.as_deref(),
            Some("Teams"),
            "a glancing overlap should not attribute"
        );
    }

    #[test]
    fn a_source_with_no_speakers_at_all_adds_nothing() {
        // Otter without diarization enabled, for instance. It should not strip
        // what the base already knew.
        let anonymous: Vec<SourceSegment> = teams()
            .iter()
            .map(|s| seg(s.start_ms, s.end_ms, None, &s.text))
            .collect();

        let (merged, report) = merge_speakers(&note67(), &anonymous, "Otter");
        assert_eq!(report.segments_named, 0);
        assert_eq!(merged[0].speaker.as_deref(), Some("You"));
    }

    #[test]
    fn an_empty_base_merges_to_nothing() {
        let (merged, report) = merge_speakers(&[], &teams(), "Teams");
        assert!(merged.is_empty());
        assert!(report.rejected, "nothing to align against");
    }
}
