//! Opt-in, one-shot model calls for configured chat inference roles.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tinyinference_llm::message::Message;
use tinyinference_llm::model::{ChatModel, ModelRequest, ModelResponse, ProviderError};
use tokio::sync::{Mutex, OnceCell};

use crate::config::Config;
use crate::core::runtime::DispatchCtx;
use crate::doctor::{DoctorCheck, DoctorCtx, DoctorTier, Finding, FindingSeverity};
use crate::inference::provider::factory::{
    create_chat_model_with_identity_in_for_inspection, InspectionRequestIdentity,
    PreparedInspectionChatModel,
};

const CATEGORY: &str = "live inference";
const CHAT_ROLES: &[&str] = &[
    "chat",
    "reasoning",
    "agentic",
    "burst",
    "coding",
    "vision",
    "memory",
    "summarization",
];
const LIVE_OUTPUT_LIMIT: u32 = 5;
const MODEL_TIMEOUT: Duration = Duration::from_secs(45);

/// Creates one report-local probe cache shared by the role-specific findings.
pub(super) fn for_context(_ctx: &DoctorCtx) -> Vec<Arc<dyn DoctorCheck>> {
    let shared = Arc::new(LiveProbeCache::default());
    CHAT_ROLES
        .iter()
        .map(|role| {
            Arc::new(InferenceLiveCheck {
                id: format!("inference/live/{role}"),
                role,
                cache: Arc::clone(&shared),
            }) as Arc<dyn DoctorCheck>
        })
        .collect()
}

struct InferenceLiveCheck {
    id: String,
    role: &'static str,
    cache: Arc<LiveProbeCache>,
}

#[async_trait]
impl DoctorCheck for InferenceLiveCheck {
    fn id(&self) -> &str {
        &self.id
    }

    fn category(&self) -> &str {
        CATEGORY
    }

    fn tier(&self) -> DoctorTier {
        DoctorTier::Live
    }

    fn timeout(&self) -> Duration {
        DoctorTier::Live.timeout()
    }

    async fn detect(&self, ctx: &DoctorCtx) -> Vec<Finding> {
        let check_id = self.id();
        let selected_config = ctx.config().clone();
        let dispatch = inspection_dispatch_context(&selected_config);
        let core = dispatch
            .core()
            .expect("inspection dispatch always has a CoreContext")
            .clone();
        let built = match crate::core::runtime::CoreContext::scope(core, async {
            create_chat_model_with_identity_in_for_inspection(
                &dispatch,
                self.role,
                &selected_config,
                0.0,
            )
        })
        .await
        {
            Ok(built) => built,
            Err(error) => {
                return vec![inspection_construction_error(
                    check_id,
                    self.role,
                    ctx.config(),
                    &error,
                )];
            }
        };

        vec![probe_prepared_model(&self.cache, built, check_id).await]
    }
}

pub(super) async fn probe_prepared_model(
    cache: &LiveProbeCache,
    built: PreparedInspectionChatModel,
    check_id: &str,
) -> Finding {
    if built.output_budget
        == crate::inference::provider::factory::InspectionOutputBudgetCapability::Unsupported
    {
        return Finding::skipped(
            check_id,
            "The Codex OAuth Responses API cannot enforce the required output-token limit; no completion request was sent.",
            "Use a provider route that supports a bounded output-token request to run this live check.",
        );
    }
    let slot = cache.slot(built.identity).await;
    let model = built.model;
    let model_id = built.model_id;
    slot.get_or_init(|| async move { invoke_once(model, model_id).await })
        .await
        .clone()
        .finding(check_id)
}

fn inspection_dispatch_context(config: &Config) -> DispatchCtx {
    match DispatchCtx::ambient().resolved() {
        Some(parent) => {
            let overlay = crate::core::runtime::ContextOverlay::new(
                config.clone(),
                parent.domains(),
                parent.tool_groups(),
            );
            DispatchCtx::new(parent.derive_for_inspection(overlay))
        }
        None => DispatchCtx::new(crate::core::runtime::CoreContext::for_inspection(
            config.clone(),
        )),
    }
}

#[derive(Default)]
pub(super) struct LiveProbeCache {
    calls: Mutex<HashMap<InspectionRequestIdentity, Arc<OnceCell<ProbeOutcome>>>>,
}

impl LiveProbeCache {
    async fn slot(&self, identity: InspectionRequestIdentity) -> Arc<OnceCell<ProbeOutcome>> {
        let mut calls = self.calls.lock().await;
        Arc::clone(
            calls
                .entry(identity)
                .or_insert_with(|| Arc::new(OnceCell::new())),
        )
    }
}

#[derive(Clone)]
struct ProbeOutcome {
    severity: FindingSeverity,
    message: String,
    fix_hint: String,
}

impl ProbeOutcome {
    fn finding(&self, id: &str) -> Finding {
        Finding::new(
            id,
            self.severity,
            self.message.clone(),
            self.fix_hint.clone(),
        )
    }
}

async fn invoke_once(model: Arc<dyn ChatModel<()>>, configured_model: String) -> ProbeOutcome {
    let request =
        ModelRequest::new(vec![Message::user("Reply OK")]).with_max_tokens(LIVE_OUTPUT_LIMIT);
    let started = Instant::now();
    let response = tokio::time::timeout(MODEL_TIMEOUT, model.invoke(&(), request)).await;
    let latency_ms = elapsed_ms(started.elapsed());
    match response {
        Ok(Ok(response)) => success_outcome(&response, &configured_model, latency_ms),
        Ok(Err(error)) => provider_error_outcome(&error),
        Err(_) => ProbeOutcome {
            severity: FindingSeverity::Error,
            message:
                "The configured inference provider did not answer before the live-check timeout."
                    .into(),
            fix_hint: "Check the provider's availability and network connection, then rerun doctor with --live.".into(),
        },
    }
}

fn success_outcome(
    response: &ModelResponse,
    configured_model: &str,
    latency_ms: u64,
) -> ProbeOutcome {
    if response.text().trim().is_empty() {
        return ProbeOutcome {
            severity: FindingSeverity::Error,
            message: "The configured inference provider returned an empty completion.".into(),
            fix_hint:
                "Check that the configured model can return text, then rerun doctor with --live."
                    .into(),
        };
    }
    let answered_model = response
        .raw
        .as_ref()
        .and_then(|raw| raw.get("model"))
        .and_then(serde_json::Value::as_str)
        .filter(|model| safe_model_id(model))
        .or_else(|| safe_model_id(configured_model).then_some(configured_model))
        .unwrap_or("configured model");
    ProbeOutcome {
        severity: FindingSeverity::Pass,
        message: format!(
            "Live completion succeeded in {latency_ms} ms using model `{answered_model}`."
        ),
        fix_hint: "No action required.".into(),
    }
}

fn provider_error_outcome(error: &tinyinference_llm::Error) -> ProbeOutcome {
    match error {
        tinyinference_llm::Error::Provider(provider_error) => {
            classify_provider_error(provider_error)
        }
        _ => ProbeOutcome {
            severity: FindingSeverity::Error,
            message: "The configured inference provider is unavailable for a live completion."
                .into(),
            fix_hint: "Check the configured provider, its local credential, and network availability, then rerun doctor with --live.".into(),
        },
    }
}

fn classify_provider_error(error: &ProviderError) -> ProbeOutcome {
    match error.status {
        Some(401 | 403) => ProbeOutcome {
            severity: FindingSeverity::Error,
            message: format!(
                "Inference authentication failed (HTTP {}).",
                error.status.unwrap()
            ),
            fix_hint: "Reconnect the configured provider or replace its API key, then rerun doctor with --live.".into(),
        },
        Some(429) => ProbeOutcome {
            severity: FindingSeverity::Error,
            message: "The configured inference provider reported a quota or rate limit (HTTP 429)."
                .into(),
            fix_hint: "Check the provider's quota or rate limit and retry when it allows another request.".into(),
        },
        Some(status) if status >= 500 => ProbeOutcome {
            severity: FindingSeverity::Error,
            message: format!("The configured inference provider is unavailable (HTTP {status})."),
            fix_hint: "Check the provider's service status and network connection, then rerun doctor with --live.".into(),
        },
        _ => ProbeOutcome {
            severity: FindingSeverity::Error,
            message: "The configured inference provider is unavailable for a live completion."
                .into(),
            fix_hint: "Check the configured model and provider endpoint, then rerun doctor with --live.".into(),
        },
    }
}

fn inspection_construction_error(
    id: &str,
    role: &str,
    config: &Config,
    error: &anyhow::Error,
) -> Finding {
    // The inspection factory's credential errors are deliberately safe,
    // typed failure text. Other construction errors may contain endpoint or
    // provider data, so never copy their text to the report.
    let detail = error.to_string().to_ascii_lowercase();
    let (message, fix_hint) = if detail.contains("credential") || detail.contains("session") {
        (
            "The configured inference credential is unavailable for a live completion.",
            "Reconnect the configured provider or provide an explicit API credential, then rerun doctor with --live.",
        )
    } else if is_external_cli_route(role, config) {
        (
            "This CLI-backed inference route has no explicit credential that can be frozen for inspection.",
            "Configure an explicit provider API key or choose a provider with a frozen inspection credential, then rerun doctor with --live.",
        )
    } else {
        (
            "The configured inference model could not be prepared for a live completion.",
            "Check the provider and model configuration, then rerun doctor with --live.",
        )
    };
    Finding::error(id, message, fix_hint)
}

fn is_external_cli_route(role: &str, config: &Config) -> bool {
    let provider = crate::inference::provider::factory::provider_for_role(role, config);
    let provider = provider.trim();
    provider == "claude_agent_sdk"
        || provider.starts_with("claude_agent_sdk:")
        || provider.starts_with("claude-code:")
}

fn safe_model_id(model: &str) -> bool {
    !model.is_empty()
        && model.len() <= 200
        && !model.contains("://")
        && model.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/' | b':')
        })
}

fn elapsed_ms(duration: Duration) -> u64 {
    duration.as_millis().min(u64::MAX as u128) as u64
}
