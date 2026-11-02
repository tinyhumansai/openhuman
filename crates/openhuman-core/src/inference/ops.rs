//! JSON-RPC controller operations for inference.

mod models;
mod oauth;
mod sentiment;
mod status;

#[cfg(test)]
use models::is_unknown_provider_user_config;
pub use models::{
    inference_get_client_config, inference_list_models, inference_update_local_settings,
    inference_update_model_settings,
};
pub use oauth::inference_provider_auth_errors;
pub use sentiment::inference_analyze_sentiment;
#[cfg(test)]
use status::expected_test_provider_model_error_kind;
pub use status::InferenceTestChatModelResult;
pub use status::{
    inference_diagnostics, inference_prompt, inference_status, inference_summarize,
    inference_test_provider_model, inference_vision_prompt,
};

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
