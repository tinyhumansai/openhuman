//! Host compatibility path for TinyAgents' provider-neutral chat-template
//! rejection classifier.
//!
//! OpenHuman retains the user-facing error mapping in `web_chat::web_errors`;
//! reusable recognition of LM Studio, llama.cpp, and Ollama template-engine
//! failures belongs to the default inference driver.

pub use tinyinference::providers::openai::is_chat_template_rejection_message;
