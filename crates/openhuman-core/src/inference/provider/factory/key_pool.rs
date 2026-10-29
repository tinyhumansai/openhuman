//! Runtime credential pool shared by native BYOK model instances.
//!
//! Parsing mirrors Penguin's multi-key support; selection and outcomes are
//! maintained in Rust, where requests actually execute. This module never
//! exposes or logs secrets. Pool identity includes the local workspace.
use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasher, RandomState};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const DEFAULT_COOLDOWN: Duration = Duration::from_secs(60);
const MAX_COOLDOWN: Duration = Duration::from_secs(300);

/// Progressive cooldown adopted from 9Router's account fallback design, while
/// preserving OpenHuman's existing 60s first-rate-limit policy. Provider
/// Retry-After wins when supplied and is capped to prevent years-long lockout.
fn quota_cooldown(streak: u32, retry_after_ms: Option<u64>) -> Duration {
    if let Some(retry_ms) = retry_after_ms {
        return Duration::from_millis(retry_ms).min(MAX_COOLDOWN);
    }
    let shift = streak.saturating_sub(1).min(3);
    DEFAULT_COOLDOWN
        .saturating_mul(1 << shift)
        .min(MAX_COOLDOWN)
}
const SESSION_BINDING_TTL: Duration = Duration::from_secs(30 * 60);
const MAX_STICKY_RUN_ID_BYTES: usize = 256;

#[derive(Clone)]
struct KeyHealth {
    id: u64,
    cooldown_until: Option<Instant>,
    evicted: bool,
    successes: u64,
    failures: u64,
    active: usize,
    last_used: Option<Instant>,
    consecutive_successes: u64,
    consecutive_failures: u64,
    /// Rate-limit-only streak for progressive quota backoff. General model
    /// errors must not lengthen the next 429 cooldown.
    consecutive_rate_limits: u32,
}
impl KeyHealth {
    fn new(id: u64) -> Self {
        Self {
            id,
            cooldown_until: None,
            evicted: false,
            successes: 0,
            failures: 0,
            active: 0,
            last_used: None,
            consecutive_successes: 0,
            consecutive_failures: 0,
            consecutive_rate_limits: 0,
        }
    }
    fn available(&self, now: Instant) -> bool {
        !self.evicted && self.cooldown_until.is_none_or(|until| until <= now)
    }
}

#[derive(Default)]
struct Pool {
    keys: Vec<KeyHealth>,
    next: usize,
    sticky: HashMap<String, (u64, Instant)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyFailure {
    Authentication,
    RateLimited,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RotationStrategy {
    #[default]
    RoundRobin,
    LeastBusy,
    PriorityWeighted,
}
impl RotationStrategy {
    pub fn parse(raw: &str) -> Self {
        match raw {
            "least_busy" | "least-leases" => Self::LeastBusy,
            "priority_weighted" | "priority-weighted" => Self::PriorityWeighted,
            _ => Self::RoundRobin,
        }
    }
}

fn registry() -> &'static Mutex<HashMap<String, Pool>> {
    static POOLS: OnceLock<Mutex<HashMap<String, Pool>>> = OnceLock::new();
    POOLS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// A process-salted SipHash identifier avoids retaining a second plaintext copy
/// of every credential in the health registry. The model instances themselves
/// retain only the keys they need to authenticate.
fn secret_id(value: &str) -> u64 {
    static SALT: OnceLock<RandomState> = OnceLock::new();
    SALT.get_or_init(RandomState::new).hash_one(value)
}

/// Process-local fingerprint for non-secret routing scope components. A randomized
/// process salt avoids persisting or exposing endpoint strings in registry keys.
/// Identical endpoints remain stable within one running process.
pub(super) fn scope_fingerprint(endpoint: &str) -> u64 {
    secret_id(endpoint)
}

/// Read a secret group from comma, semicolon, or newline-separated credentials.
/// Empty segments are discarded and identical values are kept once.
pub fn parse_api_keys(raw: &str) -> Vec<String> {
    let mut keys = Vec::new();
    for segment in raw.split([',', ';', '\n', '\r']) {
        let value = segment.trim();
        if !value.is_empty() && !keys.iter().any(|key| key == value) {
            keys.push(value.to_owned());
        }
    }
    keys
}

fn sync(pool: &mut Pool, configured: &[String]) {
    if pool.keys.len() == configured.len()
        && pool
            .keys
            .iter()
            .zip(configured)
            .all(|(entry, value)| entry.id == secret_id(value))
    {
        return;
    }
    let mut previous = std::mem::take(&mut pool.keys);
    pool.keys = configured
        .iter()
        .map(|value| {
            let id = secret_id(value);
            match previous.iter().position(|entry| entry.id == id) {
                Some(index) => previous.remove(index),
                None => KeyHealth::new(id),
            }
        })
        .collect();
    pool.next = 0;
    let ids: HashSet<u64> = pool.keys.iter().map(|key| key.id).collect();
    pool.sticky.retain(|_, (id, _)| ids.contains(id));
}

/// Select without returning a still-cooling or invalid key.
/// `excluded` avoids accidentally retrying the same key within one request.
/// Dynamic score from Penguin's WeightedKeyRotator: reward recent success,
/// penalize consecutive failure and active work, never give a healthy key
/// an entirely zero chance. Selection is proportional to these scores.
fn weighted_score(key: &KeyHealth) -> f64 {
    (100.0 + key.consecutive_successes as f64 * 5.0 - key.consecutive_failures as f64 * 20.0)
        .max(1.0)
        / (1.0 + key.active as f64 * 2.0)
}

pub fn choose_key(
    scope: &str,
    configured: &[String],
    excluded: &[usize],
    strategy: RotationStrategy,
) -> Option<(usize, String)> {
    choose_key_for_run(scope, configured, excluded, strategy, None)
}

/// Correlation/run affinity is TTL-bound and advisory: an evicted, cooling or
/// already-tried key will never be selected to preserve stickiness.
pub fn choose_key_for_run(
    scope: &str,
    configured: &[String],
    excluded: &[usize],
    strategy: RotationStrategy,
    run_id: Option<&str>,
) -> Option<(usize, String)> {
    choose_key_for_run_impl(scope, configured, excluded, strategy, run_id, false)
}

/// Choose and reserve an active lease in one lock acquisition.
///
/// Model Fleet callers must use this instead of a separate `choose_key` and
/// `acquire`: with least_busy, concurrent requests would otherwise all observe
/// zero active requests and stampede onto the same credential.
pub fn choose_key_and_acquire_for_run(
    scope: &str,
    configured: &[String],
    excluded: &[usize],
    strategy: RotationStrategy,
    run_id: Option<&str>,
) -> Option<(usize, String)> {
    choose_key_for_run_impl(scope, configured, excluded, strategy, run_id, true)
}

fn choose_key_for_run_impl(
    scope: &str,
    configured: &[String],
    excluded: &[usize],
    strategy: RotationStrategy,
    run_id: Option<&str>,
    reserve_lease: bool,
) -> Option<(usize, String)> {
    // Correlation IDs are user-influenced in JSON-RPC requests. They are
    // advisory; unsafe or oversized IDs may still select a key, but must not
    // persist unbounded strings in the 30-minute affinity cache.
    let run_id = run_id.filter(|id| {
        !id.is_empty() && id.len() <= MAX_STICKY_RUN_ID_BYTES && !id.chars().any(char::is_control)
    });
    let mut pools = registry()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let pool = pools.entry(scope.to_owned()).or_default();
    sync(pool, configured);
    if pool.keys.is_empty() {
        return None;
    }
    let now = Instant::now();
    pool.sticky.retain(|_, (_, expires)| *expires > now);
    let candidates: Vec<usize> = (0..pool.keys.len())
        .filter(|index| !excluded.contains(index) && pool.keys[*index].available(now))
        .collect();
    if candidates.is_empty() {
        return None;
    }
    let pinned = run_id
        .filter(|id| !id.is_empty())
        .and_then(|id| pool.sticky.get(id))
        .and_then(|(stored, expires)| {
            if *expires <= now {
                return None;
            }
            candidates
                .iter()
                .copied()
                .find(|idx| pool.keys[*idx].id == *stored)
        });
    let selected = if let Some(pinned) = pinned {
        pinned
    } else {
        match strategy {
            RotationStrategy::RoundRobin => (0..pool.keys.len())
                .map(|offset| (pool.next + offset) % pool.keys.len())
                .find(|idx| candidates.contains(idx)),
            RotationStrategy::LeastBusy => candidates.iter().copied().min_by_key(|idx| {
                (
                    pool.keys[*idx].active,
                    pool.keys[*idx].failures,
                    pool.keys[*idx].last_used,
                )
            }),
            RotationStrategy::PriorityWeighted => {
                let weights: Vec<f64> = candidates
                    .iter()
                    .map(|idx| weighted_score(&pool.keys[*idx]))
                    .collect();
                let total: f64 = weights.iter().sum();
                let mut ticket = rand::random::<f64>() * total;
                let mut winner = *candidates.last()?;
                for (idx, weight) in candidates.iter().zip(weights.iter()) {
                    ticket -= weight;
                    if ticket <= 0.0 {
                        winner = *idx;
                        break;
                    }
                }
                Some(winner)
            }
        }?
    };
    pool.next = (selected + 1) % pool.keys.len();
    pool.keys[selected].last_used = Some(now);
    if reserve_lease {
        // Do this before unlocking so the next least-busy selection observes
        // the in-flight reservation; Lease::from_reserved owns its release.
        pool.keys[selected].active += 1;
    }
    if let Some(run) = run_id.filter(|id| !id.is_empty()) {
        // A bounded cache prevents unbounded correlation IDs in long-lived apps.
        if pool.sticky.len() >= 512 && !pool.sticky.contains_key(run) {
            // HashMap iteration order is arbitrary; evict the binding that
            // expires first so active sessions retain their affinity.
            if let Some(oldest) = pool
                .sticky
                .iter()
                .min_by_key(|(_, (_, expires))| *expires)
                .map(|(session, _)| session.clone())
            {
                pool.sticky.remove(&oldest);
            }
        }
        pool.sticky.insert(
            run.to_owned(),
            (pool.keys[selected].id, now + SESSION_BINDING_TTL),
        );
    }
    Some((selected, configured[selected].clone()))
}

pub fn next_key_excluding(
    scope: &str,
    configured: &[String],
    excluded: &[usize],
) -> Option<(usize, String)> {
    choose_key(scope, configured, excluded, RotationStrategy::RoundRobin)
}
pub fn next_key(scope: &str, configured: &[String]) -> Option<(usize, String)> {
    next_key_excluding(scope, configured, &[])
}

/// A lease is held through a full unary request or a stream's lifetime.
pub fn acquire(scope: &str, key: &str) {
    let mut pools = registry()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(entry) = pools
        .get_mut(scope)
        .and_then(|pool| pool.keys.iter_mut().find(|k| k.id == secret_id(key)))
    {
        entry.active += 1;
    }
}
pub fn release(scope: &str, key: &str) {
    let mut pools = registry()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(entry) = pools
        .get_mut(scope)
        .and_then(|pool| pool.keys.iter_mut().find(|k| k.id == secret_id(key)))
    {
        entry.active = entry.active.saturating_sub(1);
    }
}

/// Attribute outcomes to secret identity, not a positional index that may have
/// changed after a settings update while the request was running.
pub fn record_result_for(
    scope: &str,
    key: &str,
    failure: Option<KeyFailure>,
    retry_after_ms: Option<u64>,
) {
    record_result_by_id(scope, secret_id(key), failure, retry_after_ms);
}
fn record_result_by_id(
    scope: &str,
    id: u64,
    failure: Option<KeyFailure>,
    retry_after_ms: Option<u64>,
) {
    let mut pools = registry()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(entry) = pools
        .get_mut(scope)
        .and_then(|pool| pool.keys.iter_mut().find(|item| item.id == id))
    else {
        return;
    };
    match failure {
        None => {
            entry.successes += 1;
            entry.consecutive_successes += 1;
            entry.consecutive_failures = 0;
            entry.consecutive_rate_limits = 0;
            // A success from a parallel in-flight request is not evidence that
            // another request's active 429 cooldown has been lifted.
            entry.cooldown_until = entry.cooldown_until.filter(|until| *until > Instant::now());
        }
        Some(KeyFailure::Authentication) => {
            entry.failures += 1;
            entry.consecutive_failures += 1;
            entry.consecutive_successes = 0;
            entry.consecutive_rate_limits = 0;
            entry.evicted = true;
        }
        Some(KeyFailure::RateLimited) => {
            entry.failures += 1;
            entry.consecutive_failures += 1;
            entry.consecutive_successes = 0;
            entry.consecutive_rate_limits = entry.consecutive_rate_limits.saturating_add(1);
            let delay = quota_cooldown(entry.consecutive_rate_limits, retry_after_ms);
            let until = Instant::now() + delay;
            // Competing 429 responses may specify different Retry-After delays.
            // A later shorter delay must never preempt the longer deadline.
            entry.cooldown_until = Some(
                entry
                    .cooldown_until
                    .map_or(until, |previous| previous.max(until)),
            );
        }
        Some(KeyFailure::Other) => {
            entry.failures += 1;
            entry.consecutive_failures += 1;
            entry.consecutive_successes = 0;
            // Transport and request errors are not quota signals.
            entry.consecutive_rate_limits = 0;
        }
    }
}
pub fn record_result(scope: &str, key_index: usize, failure: Option<KeyFailure>) {
    let id = {
        let pools = registry()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        pools
            .get(scope)
            .and_then(|pool| pool.keys.get(key_index))
            .map(|key| key.id)
    };
    if let Some(id) = id {
        record_result_by_id(scope, id, failure, None);
    }
}

/// Safe summary used by settings/diagnostics; deliberately no masked or raw keys.
#[derive(Clone, Debug, serde::Serialize)]
pub struct KeyStats {
    pub index: usize,
    pub status: &'static str,
    pub successes: u64,
    pub failures: u64,
    pub active_leases: usize,
    pub cooldown_remaining_ms: u64,
}
pub fn snapshot(scope: &str) -> Vec<KeyStats> {
    let pools = registry()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let now = Instant::now();
    pools
        .get(scope)
        .map(|pool| {
            pool.keys
                .iter()
                .enumerate()
                .map(|(i, key)| {
                    let cooldown = key
                        .cooldown_until
                        .and_then(|until| until.checked_duration_since(now))
                        .unwrap_or_default()
                        .as_millis() as u64;
                    KeyStats {
                        index: i,
                        status: if key.evicted {
                            "evicted"
                        } else if cooldown > 0 {
                            "cooldown"
                        } else {
                            "healthy"
                        },
                        successes: key.successes,
                        failures: key.failures,
                        active_leases: key.active,
                        cooldown_remaining_ms: cooldown,
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}
pub fn health(scope: &str) -> Vec<(bool, bool, u64, u64)> {
    snapshot(scope)
        .iter()
        .map(|s| {
            (
                s.status == "evicted",
                s.status == "cooldown",
                s.successes,
                s.failures,
            )
        })
        .collect()
}
pub fn reset(scope: &str) {
    let mut pools = registry()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(pool) = pools.get_mut(scope) {
        for entry in &mut pool.keys {
            entry.evicted = false;
            entry.cooldown_until = None;
            entry.successes = 0;
            entry.failures = 0;
            entry.consecutive_failures = 0;
            entry.consecutive_successes = 0;
            entry.consecutive_rate_limits = 0;
        }
    }
}

/// Health views for all currently initialized models in a provider group.
/// The model name is derived from the scope; no token identifier is exposed.
pub fn provider_snapshots(prefix: &str) -> Vec<(String, Vec<KeyStats>)> {
    let scopes: Vec<String> = {
        let pools = registry()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        pools
            .keys()
            .filter(|name| name.starts_with(prefix))
            .cloned()
            .collect()
    };
    let mut snapshots: Vec<_> = scopes
        .into_iter()
        .map(|scope| {
            let model = scope.strip_prefix(prefix).unwrap_or_default().to_owned();
            (model, snapshot(&scope))
        })
        .collect();
    snapshots.sort_by(|a, b| a.0.cmp(&b.0));
    snapshots
}

/// Clearing a provider's state revives evicted keys without changing secrets.
pub fn reset_provider(prefix: &str) {
    let scopes: Vec<String> = {
        let pools = registry()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        pools
            .keys()
            .filter(|name| name.starts_with(prefix))
            .cloned()
            .collect()
    };
    for scope in scopes {
        reset(&scope);
    }
}

/// Drop cached health for every model of a disconnected provider, without
/// disturbing other providers in the same workspace.
pub fn forget_provider(prefix: &str) {
    let mut pools = registry()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    pools.retain(|scope, _| !scope.starts_with(prefix));
}

/// Called when a model is removed or a provider is replaced.
pub fn forget(scope: &str) {
    let mut pools = registry()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    pools.remove(scope);
}

#[cfg(test)]
#[path = "key_pool_tests.rs"]
mod tests;
