//! Host adapters served to the compiled TinyFlows module for one run.

mod agent;
mod binding;
mod code;
mod error;
mod http;
mod llm;
mod memory;
mod prompt;
mod service;
mod tier;
pub(crate) mod tools;

#[cfg(test)]
#[path = "binding_tests.rs"]
mod binding_tests;

#[cfg(test)]
pub(crate) use agent::OpenHumanAgentRunner;
#[cfg(test)]
pub(crate) use binding::CapsHostBinding;
pub(crate) use binding::{serve, CapsHostRegistration};
pub(crate) use code::OpenHumanCode;
pub(crate) use error::{EngineError, Result};
#[cfg(test)]
pub(crate) use http::resolve_http_credential;
pub(crate) use http::OpenHumanHttp;
#[cfg(test)]
pub(crate) use llm::resolve_host_model;
pub(crate) use llm::OpenHumanLlm;
pub(crate) use memory::OpenHumanMemory;
pub(crate) use prompt::model_response_to_completion_value;
pub(crate) use tier::{
    close_audit, current_tier_policy, enforce_current_node_tier_gate, enforce_node_tier_gate,
    gate_call_for_tier,
};
pub(crate) use tools::OpenHumanTools;
pub(crate) use tools::{classify_composio_action_for_tier, NATIVE_TOOL_PREFIX};

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
