//! TinySecurity's Jev judge: on by default, wherever a credential exists.
//!
//! The judge can only turn a call that needs approval into a one-off allow,
//! never override a deny, and whatever it cannot settle still goes to a person.
//! The module takes it from its init configuration and rejects a bad one, which
//! fails enforced security closed, so anything doubtful here leaves the judge
//! out instead: a missing judge costs a question, a rejected one every call.

use tinysecurity_bus::{JudgeConfig, JudgeProvider, Origin, Secret};

use crate::config::schema::{JudgeMode, PrivacyMode, SecurityJudgeDialect, SecurityJudgeRoute};
use crate::config::{Config, DecisionModel};
use crate::modules::computer_config::{provider_key, tinyhumans_bearer};

/// The least confidence, in thousandths, that a call is intended and reversible.
const THRESHOLD_MILLIS: u16 = 900;
/// The most confidence, in thousandths, that a call sends data out.
const EXFILTRATE_MILLIS: u16 = 100;
/// How long one judgement may take before the call goes to a person.
const TIMEOUT_MS: u32 = 3_000;
/// Judgements allowed per budget window, failed ones included.
const MAX_REQUESTS: u32 = 120;
/// The budget window.
const BUDGET_WINDOW_MS: u32 = 60_000;
/// Identical calls whose scores are kept.
const CACHE_ENTRIES: u16 = 256;
/// How long kept scores last.
const CACHE_TTL_MS: u32 = 300_000;
/// The key lengths the module accepts.
const KEY_LENGTHS: std::ops::RangeInclusive<usize> = 4..=4096;
/// The longest endpoint the module accepts.
const MAX_ENDPOINT_BYTES: usize = 4096;

/// The judge for `config`, if it should run and a credential for it exists.
pub(in crate::modules) async fn resolve(config: &Config) -> Option<JudgeConfig> {
    if !wanted(config) {
        return None;
    }
    let route = match &config.security_judge {
        Some(route) => route.clone(),
        None => stored_route(config).await?,
    };
    build(config, route)
}

/// Whether `config` lets the judge run at all.
fn wanted(config: &Config) -> bool {
    config.security.enforces()
        && config.security.judge == JudgeMode::On
        // The judge sends redacted call details to its endpoint.
        && config.privacy.mode != PrivacyMode::LocalOnly
}

/// The route the credentials `config` already holds give.
async fn stored_route(config: &Config) -> Option<SecurityJudgeRoute> {
    match config.computer.decision_model {
        DecisionModel::Jev => {
            let base = config.resolved_endpoints.control_plane_url.as_deref();
            match (tinyhumans_bearer(config).await, base) {
                (Some(bearer), Some(base)) => Some(SecurityJudgeRoute::tinyhumans(base, bearer)),
                _ => provider_key(config, "openrouter", "OPENROUTER_API_KEY")
                    .await
                    .map(SecurityJudgeRoute::open_router),
            }
        }
        DecisionModel::OpenJev => provider_key(config, "openjev", "OPENJEV_API_KEY")
            .await
            .map(SecurityJudgeRoute::open_jev),
        // Sage is not a dialect the judge speaks.
        DecisionModel::Sage => None,
    }
}

/// The judge on `route`, unless `config` keeps it out or the module would
/// reject it.
fn build(config: &Config, route: SecurityJudgeRoute) -> Option<JudgeConfig> {
    let usable = KEY_LENGTHS.contains(&route.api_key.len())
        && !route.endpoint.is_empty()
        && route.endpoint.len() <= MAX_ENDPOINT_BYTES;
    if !wanted(config) || !usable {
        return None;
    }
    Some(JudgeConfig {
        credential: Secret(route.api_key),
        endpoint: route.endpoint,
        provider: match route.dialect {
            SecurityJudgeDialect::OpenRouter => JudgeProvider::OpenRouter,
            SecurityJudgeDialect::OpenJev => JudgeProvider::OpenJev,
        },
        threshold_millis: THRESHOLD_MILLIS,
        exfiltrate_millis: EXFILTRATE_MILLIS,
        thresholds: Vec::new(),
        timeout_ms: TIMEOUT_MS,
        // Scheduled and remote callers have no person to fall back on.
        origins: vec![Origin::Operator, Origin::Copilot, Origin::SubAgent],
        max_requests: MAX_REQUESTS,
        budget_window_ms: BUDGET_WINDOW_MS,
        cache_entries: CACHE_ENTRIES,
        cache_ttl_ms: CACHE_TTL_MS,
    })
}

#[cfg(test)]
#[path = "judge_tests.rs"]
pub(super) mod tests;
