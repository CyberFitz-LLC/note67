//! Client for OpenAI-compatible servers (vLLM, llama.cpp server, LM Studio, TGI).
//!
//! Only the two calls the app actually makes are implemented: list the models,
//! and complete a prompt (buffered or streamed).

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::ai::provider::{openai_v1_root, LlmError, LlmModel};

// ===== Wire types =====

#[derive(Debug, Deserialize)]
struct ModelListResponse {
    data: Vec<ModelEntry>,
}

#[derive(Debug, Deserialize)]
struct ModelEntry {
    id: String,
}

#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    temperature: f32,
    stream: bool,
    /// A ceiling on the reply.
    ///
    /// Absent, a reasoning model can deliberate without limit. One did: asked
    /// for a 200-word meeting brief, it spent thousands of tokens arguing with
    /// itself about an ambiguous phrase, repeated the same sentence hundreds of
    /// times, and never produced an answer at all — and a live pane showed the
    /// deliberation, because that was the only text that came back.
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

#[derive(Debug, Serialize)]
struct ChatMessage {
    role: &'static str,
    content: ChatContent,
}

/// A message body, which is either plain text or a sequence of parts.
///
/// Untagged, because the wire format is not a choice we get to make: the
/// OpenAI-compatible shape is a bare string for text and an array of typed
/// parts once an image is involved, and a server will reject the wrong one.
#[derive(Debug, Serialize)]
#[serde(untagged)]
enum ChatContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum ContentPart {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image_url")]
    ImageUrl { image_url: ImageUrl },
}

#[derive(Debug, Serialize)]
struct ImageUrl {
    /// A `data:` URL. Images are inlined rather than hosted: this app has no
    /// server to serve them from, and a screenshot of a meeting is not
    /// something to put behind a public URL even if it did.
    url: String,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatResponseMessage,
    /// `"length"` when the reply was cut off by the token ceiling.
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatResponseMessage {
    #[serde(default)]
    content: Option<String>,
    /// Where a reasoning model puts its thinking.
    ///
    /// Read as a fallback, because a server can be configured so that *all*
    /// output lands here and `content` is never populated — vLLM with a
    /// `--reasoning-parser` whose delimiter the model never emits does exactly
    /// that. The answer is then in this field or nowhere, and reading only
    /// `content` yields an empty summary that looks like the model had nothing
    /// to say.
    ///
    /// Two spellings are in the wild: vLLM emits `reasoning`, others
    /// `reasoning_content`.
    #[serde(default)]
    reasoning: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
}

impl ChatResponseMessage {
    /// The text of the reply, preferring real content.
    ///
    /// Reasoning is only ever a stand-in. When a server populates both — the
    /// normal case for a reasoning model — the thinking is not the answer and
    /// must not be shown as one.
    fn text(self) -> Option<String> {
        let content = self.content.filter(|c| !c.trim().is_empty());
        content
            .or_else(|| self.reasoning.filter(|r| !r.trim().is_empty()))
            .or_else(|| self.reasoning_content.filter(|r| !r.trim().is_empty()))
    }
}

#[derive(Debug, Deserialize)]
struct ChatChunk {
    #[serde(default)]
    choices: Vec<ChatChunkChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChunkChoice {
    #[serde(default)]
    delta: ChatDelta,
}

#[derive(Debug, Default, Deserialize)]
struct ChatDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reasoning: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
}

// ===== Server-sent events =====

#[derive(Debug, PartialEq, Eq)]
pub enum SseEvent {
    Data(String),
    Done,
}

/// Reassembles SSE lines across network chunk boundaries.
///
/// A streamed response arrives in arbitrary byte chunks, so a single
/// `data: {...}` line is routinely split across two of them. Parsing each chunk
/// independently silently drops those tokens; this buffers the tail until a
/// newline actually arrives.
#[derive(Default)]
pub struct SseBuffer {
    pending: String,
}

impl SseBuffer {
    pub fn push(&mut self, chunk: &str) -> Vec<SseEvent> {
        self.pending.push_str(chunk);

        let mut events = Vec::new();
        while let Some(newline) = self.pending.find('\n') {
            let line: String = self.pending.drain(..=newline).collect();
            let line = line.trim_end_matches(['\n', '\r']);

            // Blank separators, and `:` comments used as keep-alives, carry no
            // payload.
            if line.is_empty() || line.starts_with(':') {
                continue;
            }

            if let Some(payload) = line.strip_prefix("data:") {
                let payload = payload.trim();
                if payload == "[DONE]" {
                    events.push(SseEvent::Done);
                } else if !payload.is_empty() {
                    events.push(SseEvent::Data(payload.to_string()));
                }
            }
            // `event:`, `id:` and `retry:` fields are not used by this API.
        }

        events
    }
}

/// Pull the token out of one streamed chunk.
///
/// Returns `None` for chunks that carry no text — role-only openers and the
/// final `finish_reason` chunk both look like this — and for anything that
/// fails to parse, since one malformed chunk should not abort a long summary.
pub fn delta_content(payload: &str) -> Option<String> {
    let chunk: ChatChunk = serde_json::from_str(payload).ok()?;
    let content = chunk.choices.into_iter().next()?.delta.content?;
    if content.is_empty() {
        None
    } else {
        Some(content)
    }
}

/// Pull the reasoning out of one streamed chunk, if it carries any.
fn delta_reasoning(payload: &str) -> Option<String> {
    let chunk: ChatChunk = serde_json::from_str(payload).ok()?;
    let delta = chunk.choices.into_iter().next()?.delta;
    delta
        .reasoning
        .or(delta.reasoning_content)
        .filter(|r| !r.is_empty())
}

/// Lets reasoning stand in for a stream that never produced any content.
///
/// The decision cannot be made per chunk: at the time a reasoning delta
/// arrives, there is no way to know whether content will follow. So reasoning
/// is held back, and only released once the stream has ended having produced
/// nothing else. A properly configured reasoning model therefore streams its
/// answer as usual and its thinking is discarded; a misconfigured one still
/// yields its answer, arriving at the end rather than progressively.
#[derive(Debug, Default)]
pub struct ReasoningFallback {
    saw_content: bool,
    held: String,
}

impl ReasoningFallback {
    /// Feed one chunk; returns text to emit now, if any.
    pub fn push(&mut self, payload: &str) -> Option<String> {
        if let Some(token) = delta_content(payload) {
            self.saw_content = true;
            self.held.clear();
            return Some(token);
        }
        if !self.saw_content && let Some(r) = delta_reasoning(payload) {
            self.held.push_str(&r);
        }
        None
    }

    /// Called once the stream ends; returns the stand-in, if one is needed.
    pub fn finish(self) -> Option<String> {
        if self.saw_content {
            return None;
        }
        let held = self.held.trim().to_string();
        if held.is_empty() { None } else { Some(held) }
    }
}

// ===== Client =====

pub struct OpenAiCompatClient {
    client: reqwest::Client,
    v1_root: String,
    api_key: Option<String>,
}

impl OpenAiCompatClient {
    pub fn new(base_url: &str, api_key: Option<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            v1_root: openai_v1_root(base_url),
            // An empty key is the same as no key; sending `Bearer ` would make
            // some servers reject the request outright.
            api_key: api_key.filter(|k| !k.trim().is_empty()),
        }
    }

    fn authorized(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.api_key {
            Some(key) => builder.bearer_auth(key),
            None => builder,
        }
    }

    fn connect_error(&self, e: reqwest::Error) -> LlmError {
        if e.is_connect() || e.is_timeout() {
            LlmError::NotRunning(self.v1_root.clone())
        } else {
            LlmError::RequestFailed(e.to_string())
        }
    }

    /// Turn a non-success status into the most actionable error available.
    async fn status_error(&self, response: reqwest::Response, model: &str) -> LlmError {
        let status = response.status();
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return LlmError::Unauthorized;
        }
        let body = response.text().await.unwrap_or_default();
        if status.as_u16() == 404 {
            return LlmError::ModelNotFound(model.to_string());
        }
        LlmError::RequestFailed(format!("Status: {}, Body: {}", status, body))
    }

    pub async fn is_running(&self) -> bool {
        let url = format!("{}/models", self.v1_root);
        match self.authorized(self.client.get(&url)).send().await {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        }
    }

    pub async fn list_models(&self) -> Result<Vec<LlmModel>, LlmError> {
        let url = format!("{}/models", self.v1_root);

        let response = self
            .authorized(self.client.get(&url))
            .send()
            .await
            .map_err(|e| self.connect_error(e))?;

        if !response.status().is_success() {
            return Err(self.status_error(response, "").await);
        }

        let list: ModelListResponse = response
            .json()
            .await
            .map_err(|e| LlmError::InvalidResponse(e.to_string()))?;

        Ok(list
            .data
            .into_iter()
            .map(|m| LlmModel {
                name: m.id,
                // The OpenAI models endpoint reports no size, and inventing one
                // would put a wrong number in the UI.
                size: None,
                modified_at: None,
            })
            .collect())
    }

    fn chat_request(
        &self,
        model: &str,
        prompt: &str,
        temperature: f32,
        stream: bool,
        max_tokens: Option<u32>,
    ) -> ChatRequest {
        ChatRequest {
            model: model.to_string(),
            // The prompt library is written as single self-contained
            // instructions, so it maps to one user message.
            messages: vec![ChatMessage {
                role: "user",
                content: ChatContent::Text(prompt.to_string()),
            }],
            temperature,
            stream,
            max_tokens,
        }
    }

    /// Ask about an image.
    ///
    /// Separate from `generate` rather than an optional argument, because a
    /// model that cannot see returns something confidently wrong rather than an
    /// error — so the caller needs to have chosen this deliberately.
    pub async fn generate_with_image(
        &self,
        model: &str,
        prompt: &str,
        image: &[u8],
        mime: &str,
        temperature: f32,
    ) -> Result<String, LlmError> {
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(image);

        let request = ChatRequest {
            model: model.to_string(),
            messages: vec![ChatMessage {
                role: "user",
                content: ChatContent::Parts(vec![
                    ContentPart::Text {
                        text: prompt.to_string(),
                    },
                    ContentPart::ImageUrl {
                        image_url: ImageUrl {
                            url: format!("data:{mime};base64,{encoded}"),
                        },
                    },
                ]),
            }],
            temperature,
            stream: false,
            max_tokens: Some(1_500),
        };

        let url = format!("{}/chat/completions", self.v1_root);
        let response = self
            .authorized(self.client.post(&url))
            .json(&request)
            .send()
            .await
            .map_err(|e| self.connect_error(e))?;

        if !response.status().is_success() {
            return Err(self.status_error(response, model).await);
        }

        let parsed: ChatResponse = response
            .json()
            .await
            .map_err(|e| LlmError::InvalidResponse(e.to_string()))?;

        parsed
            .choices
            .into_iter()
            .next()
            .and_then(|c| c.message.text())
            .ok_or_else(|| {
                LlmError::InvalidResponse(format!(
                    "{model} returned nothing for the image. Not every model can see — check \
                     that this one accepts images."
                ))
            })
    }

    pub async fn generate(
        &self,
        model: &str,
        prompt: &str,
        temperature: f32,
        max_tokens: Option<u32>,
    ) -> Result<String, LlmError> {
        let url = format!("{}/chat/completions", self.v1_root);

        let response = self
            .authorized(self.client.post(&url))
            .json(&self.chat_request(model, prompt, temperature, false, max_tokens))
            .send()
            .await
            .map_err(|e| self.connect_error(e))?;

        if !response.status().is_success() {
            return Err(self.status_error(response, model).await);
        }

        let parsed: ChatResponse = response
            .json()
            .await
            .map_err(|e| LlmError::InvalidResponse(e.to_string()))?;

        // An empty reply is a failure, not a result. Returning "" here is how a
        // model that said nothing became a summary block with nothing in it.
        let choice = parsed.choices.into_iter().next().ok_or_else(|| {
            LlmError::InvalidResponse(format!("{model} returned no choices at all"))
        })?;

        // A reply cut off by the ceiling, with no content, is deliberation that
        // never reached an answer. The reasoning fallback exists for a server
        // that puts the *answer* there, not for showing a model arguing with
        // itself — which is what a live pane displayed when this was missing.
        let truncated = choice.finish_reason.as_deref() == Some("length");
        let had_content = choice
            .message
            .content
            .as_deref()
            .is_some_and(|c| !c.trim().is_empty());

        if truncated && !had_content {
            return Err(LlmError::InvalidResponse(format!(
                "{model} ran out of room before it produced an answer — it was still thinking. \
                 A shorter prompt or a higher token limit would help."
            )));
        }

        choice.message.text().ok_or_else(|| {
            LlmError::InvalidResponse(format!(
                "{model} returned an empty reply. If it is served through vLLM with \
                 --reasoning-parser, check that the parser matches the model: a mismatched \
                 one routes the whole answer into `reasoning` and leaves `content` null."
            ))
        })
    }

    pub async fn generate_stream(
        &self,
        model: &str,
        prompt: &str,
        temperature: f32,
        max_tokens: Option<u32>,
        tx: mpsc::Sender<String>,
    ) -> Result<String, LlmError> {
        let url = format!("{}/chat/completions", self.v1_root);

        let response = self
            .authorized(self.client.post(&url))
            .json(&self.chat_request(model, prompt, temperature, true, max_tokens))
            .send()
            .await
            .map_err(|e| self.connect_error(e))?;

        if !response.status().is_success() {
            return Err(self.status_error(response, model).await);
        }

        let mut full_response = String::new();
        let mut buffer = SseBuffer::default();
        let mut fallback = ReasoningFallback::default();
        let mut stream = response.bytes_stream();

        while let Some(chunk) = stream.next().await {
            let bytes = chunk.map_err(|e| LlmError::RequestFailed(e.to_string()))?;
            let text = String::from_utf8_lossy(&bytes);

            for event in buffer.push(&text) {
                match event {
                    SseEvent::Done => {
                        if let Some(stand_in) = fallback.finish() {
                            full_response.push_str(&stand_in);
                            let _ = tx.send(stand_in).await;
                        }
                        return Ok(full_response);
                    }
                    SseEvent::Data(payload) => {
                        if let Some(token) = fallback.push(&payload) {
                            full_response.push_str(&token);
                            let _ = tx.send(token).await;
                        }
                    }
                }
            }
        }

        // The stream ended without a [DONE], which servers do. The held
        // reasoning still has to be released, or the reply is lost entirely.
        if let Some(stand_in) = fallback.finish() {
            full_response.push_str(&stand_in);
            let _ = tx.send(stand_in).await;
        }

        Ok(full_response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_whole_line_yields_one_event() {
        let mut buf = SseBuffer::default();
        assert_eq!(
            buf.push("data: {\"a\":1}\n"),
            vec![SseEvent::Data("{\"a\":1}".to_string())]
        );
    }

    #[test]
    fn a_line_split_across_chunks_is_reassembled() {
        // The failure this guards against is silent: parsing each chunk
        // independently drops the token entirely rather than erroring.
        let mut buf = SseBuffer::default();
        assert!(buf.push("data: {\"cho").is_empty());
        assert!(buf.push("ices\":[]").is_empty());
        assert_eq!(
            buf.push("}\n"),
            vec![SseEvent::Data("{\"choices\":[]}".to_string())]
        );
    }

    #[test]
    fn several_events_in_one_chunk_all_come_out() {
        let mut buf = SseBuffer::default();
        let events = buf.push("data: {\"a\":1}\n\ndata: {\"b\":2}\n\ndata: [DONE]\n\n");
        assert_eq!(
            events,
            vec![
                SseEvent::Data("{\"a\":1}".to_string()),
                SseEvent::Data("{\"b\":2}".to_string()),
                SseEvent::Done,
            ]
        );
    }

    #[test]
    fn the_done_sentinel_is_recognised() {
        let mut buf = SseBuffer::default();
        assert_eq!(buf.push("data: [DONE]\n"), vec![SseEvent::Done]);
    }

    #[test]
    fn keepalive_comments_and_blank_lines_are_ignored() {
        // Long generations idle; servers send `:` comments to hold the
        // connection open and they must not be treated as payload.
        let mut buf = SseBuffer::default();
        assert!(buf.push(": ping\n\n\n").is_empty());
    }

    #[test]
    fn carriage_returns_are_stripped() {
        let mut buf = SseBuffer::default();
        assert_eq!(
            buf.push("data: {\"a\":1}\r\n"),
            vec![SseEvent::Data("{\"a\":1}".to_string())]
        );
    }

    #[test]
    fn a_trailing_partial_line_is_held_until_it_completes() {
        let mut buf = SseBuffer::default();
        assert_eq!(
            buf.push("data: {\"a\":1}\ndata: {\"b\""),
            vec![SseEvent::Data("{\"a\":1}".to_string())]
        );
        assert_eq!(
            buf.push(":2}\n"),
            vec![SseEvent::Data("{\"b\":2}".to_string())]
        );
    }

    #[test]
    fn a_token_is_extracted_from_a_delta() {
        let payload = r#"{"choices":[{"index":0,"delta":{"content":"Hello"}}]}"#;
        assert_eq!(delta_content(payload), Some("Hello".to_string()));
    }

    #[test]
    fn a_role_only_opener_carries_no_token() {
        // vLLM opens every stream with a role-only delta.
        let payload = r#"{"choices":[{"index":0,"delta":{"role":"assistant"}}]}"#;
        assert_eq!(delta_content(payload), None);
    }

    #[test]
    fn a_finish_chunk_carries_no_token() {
        let payload = r#"{"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#;
        assert_eq!(delta_content(payload), None);
    }

    #[test]
    fn an_empty_delta_string_is_not_emitted() {
        let payload = r#"{"choices":[{"index":0,"delta":{"content":""}}]}"#;
        assert_eq!(delta_content(payload), None);
    }

    #[test]
    fn a_malformed_chunk_is_skipped_rather_than_aborting_the_stream() {
        // One bad frame should cost a token, not the whole summary.
        assert_eq!(delta_content("not json"), None);
        assert_eq!(delta_content("{}"), None);
        assert_eq!(delta_content(r#"{"choices":[]}"#), None);
    }

    #[test]
    fn unknown_fields_do_not_break_parsing() {
        // Servers add their own extensions; usage stats and logprobs are common.
        let payload = r#"{"id":"x","object":"chat.completion.chunk","usage":null,
            "choices":[{"index":0,"delta":{"content":"hi","reasoning_content":null},
            "logprobs":null,"finish_reason":null}]}"#;
        assert_eq!(delta_content(payload), Some("hi".to_string()));
    }

    #[test]
    fn an_empty_api_key_is_treated_as_absent() {
        // `Bearer ` with nothing after it is rejected outright by some servers.
        let client = OpenAiCompatClient::new("http://spark:8000", Some("   ".to_string()));
        assert!(client.api_key.is_none());
    }

    #[test]
    fn the_v1_root_is_derived_once_at_construction() {
        let client = OpenAiCompatClient::new("http://spark:8000/", None);
        assert_eq!(client.v1_root, "http://spark:8000/v1");
    }

    /// The exact reply shape captured from the Spark's vLLM on 2026-08-20,
    /// serving lightning-30b-nvfp4 with `--reasoning-parser nemotron_v3`.
    ///
    /// `content` is null and the whole answer is in `reasoning`. Reading only
    /// `content` is what produced a summary block with nothing in it.
    #[test]
    fn an_answer_that_arrives_only_as_reasoning_is_still_the_answer() {
        let body = r#"{"choices":[{"index":0,"message":{"role":"assistant","content":null,
            "refusal":null,"annotations":null,"audio":null,"function_call":null,
            "tool_calls":[],"reasoning":"Hello!"},"finish_reason":"stop"}]}"#;
        let parsed: ChatResponse = serde_json::from_str(body).expect("parses");
        let text = parsed.choices.into_iter().next().unwrap().message.text();
        assert_eq!(text, Some("Hello!".to_string()));
    }

    #[test]
    fn real_content_always_beats_reasoning() {
        // A properly configured reasoning model populates both. The thinking is
        // not the answer and must never be shown as one.
        let body = r#"{"choices":[{"message":{"content":"The deploy is blocked.",
            "reasoning":"Let me think about what they said..."}}]}"#;
        let parsed: ChatResponse = serde_json::from_str(body).expect("parses");
        let text = parsed.choices.into_iter().next().unwrap().message.text();
        assert_eq!(text, Some("The deploy is blocked.".to_string()));
    }

    #[test]
    fn the_other_spelling_is_read_too() {
        let body = r#"{"choices":[{"message":{"content":"","reasoning_content":"An answer."}}]}"#;
        let parsed: ChatResponse = serde_json::from_str(body).expect("parses");
        let text = parsed.choices.into_iter().next().unwrap().message.text();
        assert_eq!(text, Some("An answer.".to_string()));
    }

    #[test]
    fn a_reply_with_nothing_in_it_is_nothing() {
        // Must stay None so the caller can raise an error rather than save an
        // empty summary.
        let body = r#"{"choices":[{"message":{"content":null,"reasoning":"   "}}]}"#;
        let parsed: ChatResponse = serde_json::from_str(body).expect("parses");
        assert_eq!(parsed.choices.into_iter().next().unwrap().message.text(), None);
    }

    #[test]
    fn streamed_reasoning_is_held_back_until_the_stream_proves_barren() {
        let mut f = ReasoningFallback::default();
        // Reasoning arrives first and must not be emitted — content may follow.
        assert_eq!(f.push(r#"{"choices":[{"delta":{"reasoning":"thinking..."}}]}"#), None);
        assert_eq!(
            f.push(r#"{"choices":[{"delta":{"content":"The answer."}}]}"#),
            Some("The answer.".to_string())
        );
        // Content arrived, so the thinking is discarded rather than appended.
        assert_eq!(f.finish(), None);
    }

    #[test]
    fn streamed_reasoning_is_released_when_no_content_ever_comes() {
        let mut f = ReasoningFallback::default();
        assert_eq!(f.push(r#"{"choices":[{"delta":{"reasoning":"Hel"}}]}"#), None);
        assert_eq!(f.push(r#"{"choices":[{"delta":{"reasoning":"lo!"}}]}"#), None);
        // Otherwise this stream yields an empty string and the user sees a
        // blank summary.
        assert_eq!(f.finish(), Some("Hello!".to_string()));
    }

    #[test]
    fn a_stream_of_nothing_stays_nothing() {
        let mut f = ReasoningFallback::default();
        assert_eq!(f.push(r#"{"choices":[{"delta":{"role":"assistant"}}]}"#), None);
        assert_eq!(f.finish(), None);
    }

}
