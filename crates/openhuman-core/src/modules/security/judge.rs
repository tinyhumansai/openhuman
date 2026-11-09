//! TinySecurity's Jev judge: on by default, wherever the host supplies a route.
//!
//! The judge can only turn a call that needs approval into a one-off allow,
//! never override a deny, and whatever it cannot settle still goes to a person.
//! The module takes it from its init configuration and rejects a bad one, which
//! fails enforced security closed, so anything doubtful here leaves the judge
//! out instead: a missing judge costs a question, a rejected one every call.

use tinysecurity_bus::{JudgeConfig, JudgeProvider, Origin, Secret};

use crate::config::schema::{JudgeMode, PrivacyMode, SecurityJudgeDialect, SecurityJudgeRoute};
use crate::config::Config;

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

/// The judge for `config`: only a route the host supplied. A stored
/// credential is read through TinySecurity, and the judge is resolved while
/// TinySecurity loads, so reading one there waits on that very load.
// TODO(tinyhumansai/tinysecurity#54): fetch the stored credential lazily.
pub(in crate::modules) fn resolve(config: &Config) -> Option<JudgeConfig> {
    build(config, config.security_judge.clone()?)
}

/// Whether `config` lets the judge run at all.
fn wanted(config: &Config) -> bool {
    config.security.enforces()
        && config.security.judge == JudgeMode::On
        // The judge sends redacted call details to its endpoint.
        && config.privacy.mode != PrivacyMode::LocalOnly
}

/// Whether the module takes `key`.
fn key_accepted(key: &str) -> bool {
    KEY_LENGTHS.contains(&key.len()) && !key.trim().is_empty()
}

/// Whether the module's client takes `endpoint`: an absolute URL with no
/// credentials, query or fragment, over HTTPS or on a literal loopback address.
fn endpoint_accepted(endpoint: &str) -> bool {
    if endpoint.len() > MAX_ENDPOINT_BYTES {
        return false;
    }
    let Ok(url) = url::Url::parse(endpoint) else {
        return false;
    };
    let secure = match url.scheme() {
        "https" => true,
        "http" => url
            .host_str()
            .and_then(|host| {
                host.trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .ok()
            })
            .is_some_and(|address| address.is_loopback()),
        _ => false,
    };
    secure
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
}

/// The judge on `route`, unless `config` keeps it out or the module would
/// reject it.
fn build(config: &Config, route: SecurityJudgeRoute) -> Option<JudgeConfig> {
    if !wanted(config) || !key_accepted(&route.api_key) || !endpoint_accepted(&route.endpoint) {
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
