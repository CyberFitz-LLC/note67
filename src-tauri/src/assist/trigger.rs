//! Whether what was just said is worth a suggestion.
//!
//! A local, cheap, roughly-right test that gates an expensive one. It does not
//! have to be accurate — it decides whether to spend a model call, and both
//! mistakes are survivable: a missed trigger costs one suggestion, a spurious
//! one costs a request. Getting it approximately right is worth far more than
//! getting it exactly right.
//!
//! Only what **others** said can trigger anything. Suggesting a response to
//! your own sentence is nonsense, and it is the obvious bug to write here.

/// Words that open a question even without a question mark, which live
/// transcription frequently omits.
const INTERROGATIVES: &[&str] = &[
    "who", "what", "when", "where", "why", "how", "which", "can you", "could you", "do you",
    "did you", "are you", "have you", "is there", "would you", "will you", "should we", "any chance",
];

/// Phrases that are not questions but are worth answering — a stated problem
/// is the opening a suggestion exists for.
const OPENINGS: &[&str] = &[
    "we need", "we're looking", "we are looking", "the problem", "our issue", "struggling with",
    "we were fined", "we got hit", "concerned about", "worried about", "we have to",
];

/// Should a suggestion pass run for this text?
///
/// `text` is what the others have said recently, already filtered to their
/// track by the caller.
pub fn worth_suggesting(text: &str) -> bool {
    let t = text.trim().to_lowercase();
    // Too short to carry a question or a problem, and usually an
    // acknowledgement — "yes", "mm", "right".
    if t.split_whitespace().count() < 4 {
        return false;
    }
    if t.contains('?') {
        return true;
    }
    INTERROGATIVES.iter().any(|w| {
        t.starts_with(w) || t.contains(&format!(". {w}")) || t.contains(&format!(", {w}"))
    }) || OPENINGS.iter().any(|w| t.contains(w))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_question_mark_is_enough() {
        assert!(worth_suggesting("Do you have SOC 2 certification?"));
    }

    #[test]
    fn a_question_without_punctuation_still_counts() {
        // Live transcription drops question marks constantly, so requiring one
        // would miss most of the questions this exists to catch.
        assert!(worth_suggesting("how do you handle data retention"));
        assert!(worth_suggesting("can you walk us through the pricing"));
    }

    #[test]
    fn a_stated_problem_is_an_opening() {
        // Not a question, and the single most useful thing to react to.
        assert!(worth_suggesting("We were fined last year after an audit finding"));
        assert!(worth_suggesting("the problem is our current tool cannot do that"));
    }

    #[test]
    fn acknowledgements_do_not_spend_a_model_call() {
        assert!(!worth_suggesting("yes"));
        assert!(!worth_suggesting("right, okay"));
        assert!(!worth_suggesting("mm hmm"));
        assert!(!worth_suggesting(""));
    }

    #[test]
    fn ordinary_statements_do_not_trigger() {
        // The pane earns attention by being occasional. Firing on everything is
        // how it becomes noise.
        assert!(!worth_suggesting(
            "So we rolled that out across the northern sites in March"
        ));
    }

    #[test]
    fn a_question_later_in_the_sentence_is_found() {
        assert!(worth_suggesting(
            "That makes sense. what would migration look like for us"
        ));
    }
}
