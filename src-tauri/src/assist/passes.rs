//! What the two panes ask a model, and how its answers are read.
//!
//! Both prompts are written against one fact the transcript gives us free:
//! every line is labelled `You` or `Others`, from which socket heard it. That
//! is what lets a suggestion know the difference between a question put to you
//! and a point you already made.

use serde::Deserialize;

use crate::assist::memory::Memory;

/// A line of the meeting, as the model is shown it.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub speaker: String,
    pub text: String,
}

/// Render lines for a prompt, keeping who said what.
pub fn transcript_block(lines: &[Line]) -> String {
    lines
        .iter()
        .map(|l| format!("{}: {}", l.speaker, l.text.trim()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Update the running brief.
///
/// Given the previous brief and only what is new, rather than the whole
/// meeting. A transcript grows without bound, and re-reading it every minute
/// costs more each time to say less each time.
pub fn brief_prompt(previous: Option<&str>, new_lines: &[Line]) -> String {
    let heard = transcript_block(new_lines);
    match previous {
        Some(prev) if !prev.trim().is_empty() => format!(
            "You are keeping a running brief of a meeting in progress.\n\n\
             Here is the brief so far:\n\n{prev}\n\n\
             Here is what has been said since:\n\n{heard}\n\n\
             Rewrite the brief so it covers the whole meeting including the new \
             material. Keep it under 200 words. Lead with what is being decided \
             or asked for, not with the order things were said in. Drop points \
             that turned out not to matter. Write only the brief."
        ),
        _ => format!(
            "You are keeping a running brief of a meeting in progress.\n\n\
             So far:\n\n{heard}\n\n\
             Write a brief under 200 words covering what is being discussed. \
             Lead with what is being decided or asked for. Write only the brief."
        ),
    }
}

/// Ask for suggestions, given what they just said and what you already said.
pub fn suggestion_prompt(
    recent: &[Line],
    already_said: &[Line],
    memories: &[Memory],
) -> String {
    let theirs = transcript_block(recent);
    let mine = if already_said.is_empty() {
        "(you have not spoken on this yet)".to_string()
    } else {
        transcript_block(already_said)
    };
    let recalled = crate::assist::memory::as_context(memories);

    format!(
        "You are helping someone during a live meeting. They cannot read much, \
         so be brief and specific.\n\n\
         What the others just said:\n\n{theirs}\n\n\
         What this person has already said in this meeting — do not suggest \
         points they have made:\n\n{mine}{recalled}\n\n\
         Reply with JSON only, in exactly this shape:\n\
         {{\"questions_open\": [\"...\"], \"options\": [{{\"label\": \"...\", \
         \"angle\": \"...\"}}]}}\n\n\
         `questions_open` lists anything the others asked that has not been \
         answered — an empty list if there is nothing outstanding. `options` \
         gives at most three ways this person could respond: `label` is two or \
         three words for a button, `angle` is one sentence on what that \
         response would do. Ground every option in what was actually said or in \
         the records above. If you have nothing useful, return empty lists \
         rather than filling them."
    )
}

/// Expand one chosen option into something sayable.
pub fn follow_up_prompt(recent: &[Line], chosen_label: &str, chosen_angle: &str) -> String {
    format!(
        "In a live meeting, the others just said:\n\n{}\n\n\
         The person you are helping has chosen to respond along these lines: \
         \"{chosen_label}\" — {chosen_angle}\n\n\
         Give them what to say. Three short bullet points at most, in their own \
         voice, specific to what was said. No preamble.",
        transcript_block(recent)
    )
}

/// One way to respond.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Option_ {
    pub label: String,
    #[serde(default)]
    pub angle: String,
}

/// What a suggestion pass produced.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct Suggestions {
    #[serde(default)]
    pub questions_open: Vec<String>,
    #[serde(default)]
    pub options: Vec<Option_>,
    /// Set when the reply could not be read as JSON and is being shown as it
    /// came. Better than an empty pane, and never presented as options.
    #[serde(skip)]
    pub raw_fallback: Option<String>,
}

/// Read a suggestion reply.
///
/// Models wrap JSON in prose, in code fences, or think out loud first, and a
/// structured pane cannot depend on them not doing so. The object is found
/// within the reply where possible; where it is not, the text is kept as a
/// plain suggestion rather than discarded.
///
/// What this must never do is invent an option. A fabricated button in a live
/// sales call is worse than a blank pane, because it will be pressed.
pub fn parse_suggestions(reply: &str) -> Suggestions {
    let trimmed = reply.trim();
    if trimmed.is_empty() {
        return Suggestions::default();
    }

    if let Some(parsed) = extract_json(trimmed) {
        return parsed;
    }

    Suggestions {
        raw_fallback: Some(trimmed.to_string()),
        ..Default::default()
    }
}

fn extract_json(text: &str) -> std::option::Option<Suggestions> {
    if let Ok(parsed) = serde_json::from_str::<Suggestions>(text) {
        return Some(parsed);
    }
    // The first balanced object in the reply. Enough for a fenced block, a
    // preamble, or a model that explained itself before answering.
    let start = text.find('{')?;
    let mut depth = 0usize;
    for (offset, ch) in text[start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    let candidate = &text[start..start + offset + ch.len_utf8()];
                    return serde_json::from_str::<Suggestions>(candidate).ok();
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(speaker: &str, text: &str) -> Line {
        Line {
            speaker: speaker.into(),
            text: text.into(),
        }
    }

    #[test]
    fn the_brief_is_given_only_what_is_new() {
        // The accumulator, not the whole transcript: re-reading a growing
        // meeting every minute costs more each time to say less each time.
        let prompt = brief_prompt(Some("They want SOC 2."), &[line("Others", "And pen tests.")]);
        assert!(prompt.contains("They want SOC 2."));
        assert!(prompt.contains("And pen tests."));
        assert!(prompt.contains("since"));
    }

    #[test]
    fn a_first_brief_has_no_previous_to_carry() {
        let prompt = brief_prompt(None, &[line("Others", "Let us start.")]);
        assert!(!prompt.contains("brief so far"));
        assert!(prompt.contains("Let us start."));
    }

    #[test]
    fn an_empty_previous_brief_is_treated_as_none() {
        let prompt = brief_prompt(Some("   "), &[line("Others", "Hello.")]);
        assert!(!prompt.contains("brief so far"));
    }

    #[test]
    fn suggestions_are_told_what_you_already_said() {
        // Otherwise it suggests the point you made five minutes ago, which is
        // how a pane teaches you to stop reading it.
        let prompt = suggestion_prompt(
            &[line("Others", "Do you have SOC 2?")],
            &[line("You", "We are SOC 2 Type II.")],
            &[],
        );
        assert!(prompt.contains("do not suggest \\\npoints they have made") || prompt.contains("do not suggest"));
        assert!(prompt.contains("We are SOC 2 Type II."));
    }

    #[test]
    fn having_said_nothing_yet_is_stated_rather_than_left_blank() {
        let prompt = suggestion_prompt(&[line("Others", "Hi.")], &[], &[]);
        assert!(prompt.contains("not spoken on this yet"));
    }

    #[test]
    fn recalled_memory_reaches_the_prompt_labelled() {
        let prompt = suggestion_prompt(
            &[line("Others", "We were fined last year.")],
            &[],
            &[Memory {
                text: "Acme were fined after an audit finding".into(),
                context: None,
            }],
        );
        assert!(prompt.contains("known, not said in this meeting"));
        assert!(prompt.contains("Acme were fined"));
    }

    #[test]
    fn a_clean_json_reply_parses() {
        let s = parse_suggestions(
            r#"{"questions_open":["Do we have SOC 2?"],
                "options":[{"label":"Answer directly","angle":"Say yes and cite the report."}]}"#,
        );
        assert_eq!(s.questions_open.len(), 1);
        assert_eq!(s.options[0].label, "Answer directly");
        assert!(s.raw_fallback.is_none());
    }

    #[test]
    fn json_inside_a_fenced_block_still_parses() {
        // Models do this constantly and a structured pane cannot depend on
        // them not doing it.
        let s = parse_suggestions(
            "Here you go:\n```json\n{\"questions_open\":[],\"options\":[{\"label\":\"Defer\",\"angle\":\"Buy time.\"}]}\n```",
        );
        assert_eq!(s.options.len(), 1);
        assert_eq!(s.options[0].label, "Defer");
    }

    #[test]
    fn a_reply_that_is_not_json_is_kept_rather_than_discarded() {
        // An empty pane says the assistant is broken. The model's actual words
        // are worth more than that.
        let s = parse_suggestions("They are asking about compliance — mention the audit.");
        assert!(s.options.is_empty(), "prose must never become buttons");
        assert_eq!(
            s.raw_fallback.as_deref(),
            Some("They are asking about compliance — mention the audit.")
        );
    }

    #[test]
    fn a_truncated_reply_never_becomes_a_button() {
        // The failure that matters: a fabricated option in a live sales call is
        // worse than a blank pane, because it will be pressed.
        let s = parse_suggestions(r#"{"questions_open":["Do we"#);
        assert!(s.options.is_empty());
        assert!(s.questions_open.is_empty());
        assert!(s.raw_fallback.is_some());
    }

    #[test]
    fn an_empty_reply_produces_nothing_at_all() {
        let s = parse_suggestions("   ");
        assert!(s.options.is_empty());
        assert!(s.raw_fallback.is_none());
    }

    #[test]
    fn who_said_what_survives_into_the_prompt() {
        let block = transcript_block(&[line("Others", "Hello"), line("You", "Hi")]);
        assert_eq!(block, "Others: Hello\nYou: Hi");
    }
}
