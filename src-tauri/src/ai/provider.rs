//! Backend-agnostic LLM access.
//!
//! Summaries originally spoke only to a local Ollama on a hardcoded port. This
//! module puts a single surface in front of both Ollama (local or remote) and
//! any OpenAI-compatible server — vLLM, llama.cpp's server, LM Studio, TGI — so
//! the command layer does not care which one is configured.
//!
//! It is an enum rather than a trait object because `async fn` in traits is not
//! `dyn`-compatible, and the alternatives (boxed futures, an `async-trait`
//! dependency) buy nothing here: the set of backends is closed and known.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::mpsc;

use crate::ai::ollama::{OllamaClient, OllamaError};
use crate::ai::openai::OpenAiCompatClient;

/// Default endpoint for a local Ollama install.
pub const DEFAULT_OLLAMA_URL: &str = "http://localhost:11434";

/// Which kind of server to talk to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    #[default]
    Ollama,
    /// Any server exposing the OpenAI `/v1` chat-completions API.
    OpenAiCompat,
}

impl ProviderKind {
    /// Parse a persisted settings value, falling back to Ollama for anything
    /// unrecognised so a hand-edited database cannot brick the AI features.
    pub fn from_setting(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "openai_compat" | "openai" | "vllm" => ProviderKind::OpenAiCompat,
            _ => ProviderKind::Ollama,
        }
    }

    pub fn as_setting(&self) -> &'static str {
        match self {
            ProviderKind::Ollama => "ollama",
            ProviderKind::OpenAiCompat => "openai_compat",
        }
    }
}

/// Where to reach the model server, and how to authenticate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfig {
    pub kind: ProviderKind,
    pub base_url: String,
    /// Sent as a bearer token. Optional: a vLLM server on a trusted LAN
    /// usually runs without one.
    pub api_key: Option<String>,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            kind: ProviderKind::Ollama,
            base_url: DEFAULT_OLLAMA_URL.to_string(),
            api_key: None,
        }
    }
}

impl ProviderConfig {
    /// Reject anything that is not an http(s) URL before it reaches reqwest,
    /// where the failure would surface much later as a confusing connect error.
    pub fn validate(&self) -> Result<(), LlmError> {
        let url = self.base_url.trim();
        if url.is_empty() {
            return Err(LlmError::InvalidConfig("The server URL is empty".into()));
        }
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err(LlmError::InvalidConfig(format!(
                "The server URL must start with http:// or https:// (got {:?})",
                url
            )));
        }
        Ok(())
    }
}

/// A model offered by whichever backend is configured.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmModel {
    pub name: String,
    /// On-disk size in bytes. Ollama reports it; the OpenAI `/v1/models`
    /// response has no equivalent, so it is absent there rather than faked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified_at: Option<String>,
}

#[derive(Error, Debug)]
pub enum LlmError {
    #[error("Cannot reach the model server at {0}. Check that it is running and the URL is correct.")]
    NotRunning(String),
    #[error("Model not found: {0}")]
    ModelNotFound(String),
    #[error("The server rejected the request as unauthorized. Check the API key.")]
    Unauthorized,
    #[error("Request failed: {0}")]
    RequestFailed(String),
    #[error("Invalid response: {0}")]
    InvalidResponse(String),
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),
}

impl From<OllamaError> for LlmError {
    fn from(e: OllamaError) -> Self {
        match e {
            OllamaError::NotRunning => LlmError::NotRunning(DEFAULT_OLLAMA_URL.to_string()),
            OllamaError::ModelNotFound(m) => LlmError::ModelNotFound(m),
            OllamaError::RequestFailed(m) => LlmError::RequestFailed(m),
            OllamaError::InvalidResponse(m) => LlmError::InvalidResponse(m),
        }
    }
}

/// Strip trailing slashes so joining a path cannot produce a double slash.
pub fn normalize_base_url(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}

/// Build the `/v1` root for an OpenAI-compatible server.
///
/// Users paste both forms — `http://spark:8000` and `http://spark:8000/v1` —
/// and both have to work, so an existing `/v1` suffix is kept rather than
/// doubled.
pub fn openai_v1_root(base_url: &str) -> String {
    let base = normalize_base_url(base_url);
    if base.ends_with("/v1") {
        base
    } else {
        format!("{}/v1", base)
    }
}

/// The configured backend.
pub enum LlmClient {
    Ollama(OllamaClient),
    OpenAiCompat(OpenAiCompatClient),
}

impl LlmClient {
    pub fn from_config(config: &ProviderConfig) -> Result<Self, LlmError> {
        config.validate()?;
        Ok(match config.kind {
            ProviderKind::Ollama => {
                LlmClient::Ollama(OllamaClient::with_base_url(&config.base_url))
            }
            ProviderKind::OpenAiCompat => LlmClient::OpenAiCompat(OpenAiCompatClient::new(
                &config.base_url,
                config.api_key.clone(),
            )),
        })
    }

    /// Whether the server answers at all.
    pub async fn is_running(&self) -> bool {
        match self {
            LlmClient::Ollama(c) => c.is_running().await,
            LlmClient::OpenAiCompat(c) => c.is_running().await,
        }
    }

    pub async fn list_models(&self) -> Result<Vec<LlmModel>, LlmError> {
        match self {
            LlmClient::Ollama(c) => Ok(c
                .list_models()
                .await?
                .into_iter()
                .map(|m| LlmModel {
                    name: m.name,
                    size: Some(m.size),
                    modified_at: Some(m.modified_at),
                })
                .collect()),
            LlmClient::OpenAiCompat(c) => c.list_models().await,
        }
    }

    pub async fn generate(
        &self,
        model: &str,
        prompt: &str,
        temperature: f32,
        context_length: Option<u32>,
    ) -> Result<String, LlmError> {
        match self {
            LlmClient::Ollama(c) => Ok(c
                .generate(model, prompt, temperature, context_length)
                .await?),
            LlmClient::OpenAiCompat(c) => c.generate(model, prompt, temperature).await,
        }
    }

    pub async fn generate_stream(
        &self,
        model: &str,
        prompt: &str,
        temperature: f32,
        context_length: Option<u32>,
        tx: mpsc::Sender<String>,
    ) -> Result<String, LlmError> {
        match self {
            LlmClient::Ollama(c) => Ok(c
                .generate_stream(model, prompt, temperature, context_length, tx)
                .await?),
            LlmClient::OpenAiCompat(c) => c.generate_stream(model, prompt, temperature, tx).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trailing_slash_does_not_become_a_double_slash() {
        assert_eq!(
            normalize_base_url("http://spark:8000/"),
            "http://spark:8000"
        );
        assert_eq!(
            normalize_base_url("  http://spark:8000///  "),
            "http://spark:8000"
        );
    }

    #[test]
    fn the_v1_suffix_is_added_when_missing() {
        assert_eq!(openai_v1_root("http://spark:8000"), "http://spark:8000/v1");
        assert_eq!(openai_v1_root("http://spark:8000/"), "http://spark:8000/v1");
    }

    #[test]
    fn an_existing_v1_suffix_is_not_doubled() {
        // Both forms get pasted out of vLLM docs and both have to work.
        assert_eq!(
            openai_v1_root("http://spark:8000/v1"),
            "http://spark:8000/v1"
        );
        assert_eq!(
            openai_v1_root("http://spark:8000/v1/"),
            "http://spark:8000/v1"
        );
    }

    #[test]
    fn a_path_prefix_is_preserved() {
        // Reverse proxies commonly mount the API under a subpath.
        assert_eq!(
            openai_v1_root("https://gw.example.com/llm"),
            "https://gw.example.com/llm/v1"
        );
    }

    #[test]
    fn provider_kind_round_trips_through_settings() {
        for kind in [ProviderKind::Ollama, ProviderKind::OpenAiCompat] {
            assert_eq!(ProviderKind::from_setting(kind.as_setting()), kind);
        }
    }

    #[test]
    fn unknown_provider_settings_fall_back_to_ollama() {
        // A hand-edited or partially-migrated database must not brick the AI
        // features; the local default is the safe landing spot.
        assert_eq!(ProviderKind::from_setting(""), ProviderKind::Ollama);
        assert_eq!(ProviderKind::from_setting("nonsense"), ProviderKind::Ollama);
    }

    #[test]
    fn common_provider_aliases_are_accepted() {
        assert_eq!(
            ProviderKind::from_setting("vllm"),
            ProviderKind::OpenAiCompat
        );
        assert_eq!(
            ProviderKind::from_setting("OpenAI"),
            ProviderKind::OpenAiCompat
        );
    }

    #[test]
    fn a_non_http_url_is_rejected_before_it_reaches_the_network() {
        let config = ProviderConfig {
            kind: ProviderKind::OpenAiCompat,
            base_url: "spark:8000".to_string(),
            api_key: None,
        };
        assert!(matches!(
            config.validate(),
            Err(LlmError::InvalidConfig(_))
        ));
    }

    #[test]
    fn an_empty_url_is_rejected() {
        let config = ProviderConfig {
            kind: ProviderKind::Ollama,
            base_url: "   ".to_string(),
            api_key: None,
        };
        assert!(matches!(
            config.validate(),
            Err(LlmError::InvalidConfig(_))
        ));
    }

    #[test]
    fn the_default_config_is_a_local_ollama() {
        let config = ProviderConfig::default();
        assert_eq!(config.kind, ProviderKind::Ollama);
        assert_eq!(config.base_url, DEFAULT_OLLAMA_URL);
        assert!(config.api_key.is_none());
        assert!(config.validate().is_ok());
    }

    #[test]
    fn a_model_without_a_size_omits_the_field_entirely() {
        // The UI hides the size label when it is absent; serialising a zero
        // would render "0 MB" next to every OpenAI-compatible model.
        let json = serde_json::to_string(&LlmModel {
            name: "gemma-4".to_string(),
            size: None,
            modified_at: None,
        })
        .unwrap();
        assert!(!json.contains("size"), "unexpected size field in {}", json);
    }
}
