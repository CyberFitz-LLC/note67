//! Recall from a memory store, for suggestions that know something.
//!
//! A suggestion built only from the last thing someone said is a paraphrase.
//! What makes one worth reading is that it knows something the conversation has
//! not mentioned yet — a previous meeting, a commitment, a product that fits
//! the problem being described. That is what this fetches.
//!
//! **Recall failing never stops a pass.** No memory means a weaker suggestion,
//! not no suggestion, so every failure here returns an empty set and says so in
//! the log. A meeting assistant that goes silent because a memory service is
//! down has chosen the worse of two outcomes.

use serde::Deserialize;

/// One thing the store knows.
#[derive(Debug, Clone, PartialEq)]
pub struct Memory {
    pub text: String,
    pub context: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RecallResponse {
    #[serde(default)]
    results: Vec<RecallResult>,
}

#[derive(Debug, Deserialize)]
struct RecallResult {
    #[serde(default)]
    text: String,
    #[serde(default)]
    context: Option<String>,
}

/// Pull what the store knows about `query`.
///
/// `base_url` and `bank` come from settings — the bank especially, because what
/// a bank contains decides whether any of this is useful, and only the person
/// who filled it knows which one to ask.
pub async fn recall(
    client: &reqwest::Client,
    base_url: &str,
    bank: &str,
    query: &str,
    max_tokens: u32,
) -> Vec<Memory> {
    if base_url.trim().is_empty() || bank.trim().is_empty() || query.trim().is_empty() {
        return Vec::new();
    }

    let url = format!(
        "{}/v1/default/banks/{}/memories/recall",
        base_url.trim_end_matches('/'),
        bank.trim()
    );

    let response = match client
        .post(&url)
        .json(&serde_json::json!({ "query": query, "max_tokens": max_tokens }))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[assist] recall unavailable ({e}); continuing without memory");
            return Vec::new();
        }
    };

    if !response.status().is_success() {
        eprintln!(
            "[assist] recall returned {}; continuing without memory",
            response.status()
        );
        return Vec::new();
    }

    match response.json::<RecallResponse>().await {
        Ok(body) => parse(body),
        Err(e) => {
            eprintln!("[assist] recall answer unreadable ({e}); continuing without memory");
            Vec::new()
        }
    }
}

fn parse(body: RecallResponse) -> Vec<Memory> {
    body.results
        .into_iter()
        .filter(|r| !r.text.trim().is_empty())
        .map(|r| Memory {
            text: r.text.trim().to_string(),
            context: r.context.filter(|c| !c.trim().is_empty()),
        })
        .collect()
}

/// Render recalled memories for a prompt.
///
/// Labelled as recall rather than folded in with the transcript, so the model
/// can tell what was said in this meeting from what is merely known. A
/// suggestion that attributes a remembered fact to something someone just said
/// is wrong in a way that is hard to notice and embarrassing to act on.
pub fn as_context(memories: &[Memory]) -> String {
    if memories.is_empty() {
        return String::new();
    }
    let lines: Vec<String> = memories
        .iter()
        .map(|m| match &m.context {
            Some(c) => format!("- ({c}) {}", m.text),
            None => format!("- {}", m.text),
        })
        .collect();
    format!(
        "\n\nFrom your own records — known, not said in this meeting:\n{}",
        lines.join("\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape the live service returns, captured from
    /// hindsight.jtpa.net on 2026-09-03.
    const REAL: &str = r#"{"results":[
        {"id":"a","text":"John runs Note67 against a Spark appliance","type":"world",
         "entities":[],"context":"note67","occurred_start":null,"mentioned_at":"2026-08-01"},
        {"id":"b","text":"The audit finding last year led to a fine","type":"world",
         "entities":[],"context":null,"mentioned_at":"2026-07-14"}
    ]}"#;

    #[test]
    fn the_real_response_shape_parses() {
        let body: RecallResponse = serde_json::from_str(REAL).expect("parses");
        let memories = parse(body);
        assert_eq!(memories.len(), 2);
        assert_eq!(memories[0].context.as_deref(), Some("note67"));
        assert_eq!(memories[1].context, None);
    }

    #[test]
    fn empty_memories_are_dropped_rather_than_shown_as_blanks() {
        let body: RecallResponse =
            serde_json::from_str(r#"{"results":[{"text":"   "},{"text":"real"}]}"#).expect("parses");
        let memories = parse(body);
        assert_eq!(memories.len(), 1);
        assert_eq!(memories[0].text, "real");
    }

    #[test]
    fn a_response_with_no_results_is_not_an_error() {
        let body: RecallResponse = serde_json::from_str(r#"{}"#).expect("parses");
        assert!(parse(body).is_empty());
    }

    #[test]
    fn recall_is_labelled_so_it_cannot_be_read_as_something_said() {
        // The failure this guards: a suggestion that attributes a remembered
        // fact to a speaker, which is hard to notice and embarrassing to act
        // on in front of the person who did not say it.
        let context = as_context(&[Memory {
            text: "They were fined after an audit finding".into(),
            context: Some("acme".into()),
        }]);
        assert!(context.contains("known, not said in this meeting"));
        assert!(context.contains("(acme)"));
    }

    #[test]
    fn no_memories_adds_nothing_to_the_prompt() {
        // Not an empty heading with nothing under it, which reads to a model as
        // "your records are empty" rather than "recall was not available".
        assert_eq!(as_context(&[]), "");
    }
}
