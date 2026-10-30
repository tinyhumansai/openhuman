//! Product-specific overrides for TinyInference's provider-neutral failure copy.

use tinyinference_llm::failure::copy::{
    failure_copy_with, FailureCopy, FailureCopyClass, FailureCopyProvider,
};

/// A failure class the chat surface renders copy for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FailureClass {
    CodexSessionExpired,
    SessionExpired,
    LocalSessionManagedUnavailable,
    ActionBudget,
    MaxIterations,
    TurnTimeout,
    DiskFull,
    EmptyResponse,
    ChatTemplateRejected,
    RateLimited,
    RateLimitedBilling,
    ManagedRateLimited,
    Timeout,
    AuthError,
    BudgetExhausted,
    ManagedBudgetExhausted,
    ProviderUnavailable,
    ManagedUnavailable,
    PayloadTooLarge,
    ContextOverflow,
    ModelConfigRejected,
    ModelUnavailable,
    CapabilityUnsupported,
    MalformedHistory,
    RequestRejected,
    ManagedRequestRejected,
    ManagedMalformedRequest,
    Network,
    Inference,
    ManagedInternal,
}

struct OpenHumanFailureCopy;

macro_rules! row {
    ($key:literal, $error_type:literal, $source:literal, $retryable:literal, $copy:literal) => {
        FailureCopy {
            key: $key,
            error_type: $error_type,
            source: $source,
            retryable: $retryable,
            copy: $copy,
        }
    };
}

impl FailureCopyProvider for OpenHumanFailureCopy {
    fn copy(&self, class: FailureCopyClass) -> Option<FailureCopy> {
        use FailureCopyClass::*;
        let row = match class {
            ChatTemplateRejected => {
                let mut value = tinyinference_llm::failure::copy::default_failure_copy(class)?;
                value.copy = "This model's chat template rejected the request — it isn't the model, the temperature, or your API key. Local models without native tool calling are driven through their own chat template, and some templates refuse the message shape of a tool step. Start a new chat to reset the history, or pick a model with native tool support in Connections → API keys → LLM.";
                value
            }
            RateLimitedBilling => {
                let mut value = tinyinference_llm::failure::copy::default_failure_copy(class)?;
                value.copy = "Your AI provider is rejecting requests for billing or plan reasons (out of credits, plan limit, or unavailable model). Retrying won't help — open Settings to top up, upgrade your plan, or pick a different model.";
                value
            }
            Authentication => {
                let mut value = tinyinference_llm::failure::copy::default_failure_copy(class)?;
                value.copy = "There's an authentication issue with the AI provider. Please check your API key in settings.";
                value
            }
            EmptyResponse => {
                let mut value = tinyinference_llm::failure::copy::default_failure_copy(class)?;
                value.copy = "The model returned an empty response. Please retry. If it keeps happening, try a different model or check its setup in Connections → API keys → LLM.";
                value
            }
            ModelConfigRejected => {
                let mut value = tinyinference_llm::failure::copy::default_failure_copy(class)?;
                value.copy = "Your AI provider rejected the request's model or temperature setting. Check your model and routing in Settings → LLM.";
                value
            }
            CapabilityUnsupported => {
                let mut value = tinyinference_llm::failure::copy::default_failure_copy(class)?;
                value.copy = "This model can't process images. Remove the attachment or switch to a vision-capable model in Connections → API keys → LLM.";
                value
            }
            RequestRejected => {
                let mut value = tinyinference_llm::failure::copy::default_failure_copy(class)?;
                value.copy = "The AI provider rejected the request — this is usually a model or parameter incompatibility. Try a different model in Connections → API keys → LLM.";
                value
            }
            HostDefined("codex_session_expired") => row!("chat_error.codex_session_expired", "provider_error", "auth", false, "Your Codex session has expired. Please reconnect it in Settings → Integrations."),
            HostDefined("session_expired") => row!("chat_error.session_expired", "session_expired", "auth", false, "Your OpenHuman session has expired. Please sign in again to continue."),
            HostDefined("local_session_managed_unavailable") => row!("chat_error.local_session_managed_unavailable", "auth_error", "config", false, "You're on the local offline profile, which has no OpenHuman account behind it, so the managed (cloud) model can't run. Sign in to use managed models, or switch routing to \"Use Your Own Models\" in Connections → API keys → LLM and add your own provider."),
            HostDefined("action_budget") => row!("chat_error.action_budget", "action_budget_exceeded", "openhuman_budget", true, "You've hit OpenHuman's per-hour action budget — this is a local safety cap, not your AI provider. The window decays gradually; you can keep chatting in this thread and tool-heavy steps will resume as the budget refills."),
            HostDefined("max_iterations") => row!("chat_error.max_iterations", "max_iterations", "agent_loop", true, "The agent ran the maximum number of tool steps for one turn without finishing. This usually means a tool kept failing (often a rate limit on a web fetch). You can retry the same question in this thread once the underlying limit clears."),
            HostDefined("turn_timeout") => row!("chat_error.turn_timeout", "turn_timeout", "agent_loop", true, "This turn ran past its time budget without finishing and was stopped so it wouldn't hang. This usually means a tool call or a delegated sub-agent stalled. You can retry your question in this thread."),
            HostDefined("disk_full") => row!("chat_error.disk_full", "disk_full", "local_storage", true, "Your disk is full, so this conversation couldn't be saved. Free up some disk space, then send your message again in this thread."),
            HostDefined("managed_rate_limited") => row!("chat_error.managed_rate_limited", "rate_limited", "provider", true, "Your AI provider is rate-limiting requests. You can retry in this thread."),
            HostDefined("budget_exhausted") => row!("chat_error.budget_exhausted", "budget_exhausted", "openhuman_billing", false, "You're out of credits, so I can't run the managed (cloud) model right now. You can top up your credits or pick a plan to continue — or, if you've enabled a local model like Ollama, switch routing to \"Use Your Own Models\" in Connections → API keys → LLM."),
            HostDefined("managed_budget_exhausted") => row!("chat_error.managed_budget_exhausted", "budget_exhausted", "openhuman_billing", false, "You're out of credits. Top up, or switch to 'Use Your Own Models' in Settings."),
            HostDefined("managed_unavailable") => row!("chat_error.managed_unavailable", "provider_error", "provider", true, "The AI service is temporarily unavailable — we've been notified. Please try again shortly."),
            HostDefined("malformed_history") => row!("chat_error.malformed_history", "provider_request_rejected", "provider", true, "We hit a temporary glitch in this conversation — we've cleared it. Please send your message again."),
            HostDefined("managed_request_rejected") => row!("chat_error.managed_request_rejected", "provider_request_rejected", "provider", false, "The request was rejected — usually a model or parameter mismatch. Try a different model in Connections → API keys → LLM."),
            HostDefined("managed_malformed_request") => row!("chat_error.managed_malformed_request", "provider_request_rejected", "provider", false, "Something went wrong with this message. Try rephrasing it — or start a new thread if it keeps happening."),
            HostDefined("managed_internal") => row!("chat_error.managed_internal", "inference", "provider", true, "Something went wrong — we've been notified. Please try again."),
            HostDefined(_) => return None,
            _ => return None,
        };
        Some(row)
    }
}

/// Maps the stable chat-surface class to the owner class and applies product
/// overrides before TinyInference's provider-neutral default.
pub(crate) fn failure_copy(class: FailureClass) -> FailureCopy {
    use FailureClass::*;
    let owner_class = match class {
        CodexSessionExpired => FailureCopyClass::HostDefined("codex_session_expired"),
        SessionExpired => FailureCopyClass::HostDefined("session_expired"),
        LocalSessionManagedUnavailable => {
            FailureCopyClass::HostDefined("local_session_managed_unavailable")
        }
        ActionBudget => FailureCopyClass::HostDefined("action_budget"),
        MaxIterations => FailureCopyClass::HostDefined("max_iterations"),
        TurnTimeout => FailureCopyClass::HostDefined("turn_timeout"),
        DiskFull => FailureCopyClass::HostDefined("disk_full"),
        EmptyResponse => FailureCopyClass::EmptyResponse,
        ChatTemplateRejected => FailureCopyClass::ChatTemplateRejected,
        RateLimited => FailureCopyClass::RateLimited,
        RateLimitedBilling => FailureCopyClass::RateLimitedBilling,
        ManagedRateLimited => FailureCopyClass::HostDefined("managed_rate_limited"),
        Timeout => FailureCopyClass::Timeout,
        AuthError => FailureCopyClass::Authentication,
        BudgetExhausted => FailureCopyClass::HostDefined("budget_exhausted"),
        ManagedBudgetExhausted => FailureCopyClass::HostDefined("managed_budget_exhausted"),
        ProviderUnavailable => FailureCopyClass::ProviderUnavailable,
        ManagedUnavailable => FailureCopyClass::HostDefined("managed_unavailable"),
        PayloadTooLarge => FailureCopyClass::PayloadTooLarge,
        ContextOverflow => FailureCopyClass::ContextOverflow,
        ModelConfigRejected => FailureCopyClass::ModelConfigRejected,
        ModelUnavailable => FailureCopyClass::ModelUnavailable,
        CapabilityUnsupported => FailureCopyClass::CapabilityUnsupported,
        MalformedHistory => FailureCopyClass::HostDefined("malformed_history"),
        RequestRejected => FailureCopyClass::RequestRejected,
        ManagedRequestRejected => FailureCopyClass::HostDefined("managed_request_rejected"),
        ManagedMalformedRequest => FailureCopyClass::HostDefined("managed_malformed_request"),
        Network => FailureCopyClass::Network,
        Inference => FailureCopyClass::Inference,
        ManagedInternal => FailureCopyClass::HostDefined("managed_internal"),
    };
    failure_copy_with(&OpenHumanFailureCopy, owner_class)
        .expect("OpenHuman overrides all product-defined failure classes")
}
