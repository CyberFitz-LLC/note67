pub mod ollama;
pub mod openai;
pub mod prompts;
pub mod provider;

pub use prompts::{SummaryPrompts, WritingPrompts};
pub use provider::{LlmClient, LlmModel, ProviderConfig, ProviderKind, DEFAULT_OLLAMA_URL};
