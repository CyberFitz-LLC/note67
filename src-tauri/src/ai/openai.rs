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
}

#[derive(Debug, Serialize)]
struct ChatMessage {
    role: &'static str,
    content: String,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatResponseMessage,
}

#[derive(Debug, Deserialize)]
struct ChatResponseMessage {
    #[serde(default)]
    content: Option<String>,
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

    fn chat_request(&self, model: &str, prompt: &str, temperature: f32, stream: bool) -> ChatRequest {
        ChatRequest {
            model: model.to_string(),
            // The prompt library is written as single self-contained
            // instructions, so it maps to one user message.
            messages: vec![ChatMessage {
                role: "user",
                content: prompt.to_string(),
            }],
            temperature,
            stream,
        }
    }

    pub async fn generate(
        &self,
        model: &str,
        prompt: &str,
        temperature: f32,
    ) -> Result<String, LlmError> {
        let url = format!("{}/chat/completions", self.v1_root);

        let response = self
            .authorized(self.client.post(&url))
            .json(&self.chat_request(model, prompt, temperature, false))
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

        Ok(parsed
            .choices
            .into_iter()
            .next()
            .and_then(|c| c.message.content)
            .unwrap_or_default())
    }

    pub async fn generate_stream(
        &self,
        model: &str,
        prompt: &str,
        temperature: f32,
        tx: mpsc::Sender<String>,
    ) -> Result<String, LlmError> {
        let url = format!("{}/chat/completions", self.v1_root);

        let response = self
            .authorized(self.client.post(&url))
            .json(&self.chat_request(model, prompt, temperature, true))
            .send()
            .await
            .map_err(|e| self.connect_error(e))?;

        if !response.status().is_success() {
            return Err(self.status_error(response, model).await);
        }

        let mut full_response = String::new();
        let mut buffer = SseBuffer::default();
        let mut stream = response.bytes_stream();

        while let Some(chunk) = stream.next().await {
            let bytes = chunk.map_err(|e| LlmError::RequestFailed(e.to_string()))?;
            let text = String::from_utf8_lossy(&bytes);

            for event in buffer.push(&text) {
                match event {
                    SseEvent::Done => return Ok(full_response),
                    SseEvent::Data(payload) => {
                        if let Some(token) = delta_content(&payload) {
                            full_response.push_str(&token);
                            let _ = tx.send(token).await;
                        }
                    }
                }
            }
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
}
