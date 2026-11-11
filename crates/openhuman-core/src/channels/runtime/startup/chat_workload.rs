//! Resolving which chat provider the channel runtime should build.

use crate::config::Config;
use crate::inference::provider;

/// How the channels runtime should construct its default chat provider.
///
/// Issue #3098 sub-issue 1: the runtime used to ignore the per-workload
/// `chat_provider` routing and unconditionally build a cloud chain, so
/// Telegram (and other channels) never honored a user's local-Ollama /
/// BYOK selection. `resolve_chat_workload` inspects the resolved chat
/// workload string and chooses between the managed-cloud selection (Cloud)
/// and dispatching to the unified workload factory (Workload).
pub(super) enum ChatWorkloadResolution {
    /// Preserve the managed-cloud selection and `config.default_model`.
    Cloud,
    /// Build the channel provider via `create_chat_provider("chat", config)`.
    Workload {
        provider_string: String,
        slug: String,
    },
}

pub(super) fn resolve_chat_workload(config: &Config) -> ChatWorkloadResolution {
    let resolved = provider::provider_for_role("chat", config);
    let trimmed = resolved.trim();
    if trimmed.is_empty()
        || trimmed == "cloud"
        || trimmed == provider::factory::INFERENCE_BACKEND_ID
    {
        return ChatWorkloadResolution::Cloud;
    }
    let slug = trimmed
        .split_once(':')
        .map(|(s, _)| s.to_string())
        .unwrap_or_else(|| trimmed.to_string());
    ChatWorkloadResolution::Workload {
        provider_string: trimmed.to_string(),
        slug,
    }
}
