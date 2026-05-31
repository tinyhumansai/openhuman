//! ChatGPT / OpenAI Codex subscription OAuth for the `openai` cloud provider slug.

mod config;
mod flow;
mod store;

#[cfg(test)]
#[path = "flow_tests.rs"]
mod tests;

pub use flow::{
    complete_openai_oauth, disconnect_openai_oauth, import_openai_oauth_from_codex_cli,
    openai_oauth_status, start_openai_oauth,
};
pub use store::{
    lookup_openai_bearer_token, lookup_openai_oauth_credentials, OPENAI_OAUTH_PROFILE_NAME,
    OPENAI_PROVIDER_KEY,
};
