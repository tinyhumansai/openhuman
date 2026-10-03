//! Builds and fingerprints the cached session `Agent`: target agent
//! resolution, model-override normalization, locale reply directive, and the
//! `SessionCacheFingerprint` that decides whether a cached agent can be
//! reused for the next turn on a thread.

use crate::agent::OpenHumanSessionHost;
use crate::config::Config;
use serde_json::json;

use super::types::{SessionCacheFingerprint, SessionEntry};

pub(super) fn autonomy_signature(config: &Config) -> String {
    serde_json::to_string(&config.autonomy).unwrap_or_default()
}

/// Signature of `config.model_registry` for the session-cache fingerprint.
/// Captures every per-model `vision` flag so toggling one in Settings forces a
/// rebuild (picking up the new build-time `model_vision`). Mirrors
/// [`autonomy_signature`].
pub(super) fn model_registry_signature(config: &Config) -> String {
    serde_json::to_string(&config.model_registry).unwrap_or_default()
}

/// The agent a web-chat turn runs as: `[agent] chat_agent_id` when an operator
/// set one, `orchestrator` otherwise.
///
/// The parameter was threaded in and ignored, so this path was pinned to the
/// orchestrator and its definition's `max_iterations` — no config could move
/// it, because a definition cap *overwrites* `agent.max_tool_iterations` rather
/// than being bounded by it (`session_host::builder::factory`). An unknown or
/// blank id falls back rather than failing the turn: the registry answers for
/// `orchestrator` on every install, and a typo in an optional setting should
/// not take chat down.
pub(super) fn pick_target_agent_id(config: &Config) -> String {
    const DEFAULT_CHAT_AGENT_ID: &str = "orchestrator";
    let selected = config
        .agent
        .chat_agent_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .unwrap_or(DEFAULT_CHAT_AGENT_ID);

    if OpenHumanSessionHost::is_runnable_agent_id(config, selected) {
        return selected.to_string();
    }

    log::warn!(
        "[web-channel] configured chat_agent_id={selected:?} is not a runnable definition; falling back to {DEFAULT_CHAT_AGENT_ID}"
    );
    DEFAULT_CHAT_AGENT_ID.to_string()
}

pub(crate) fn normalize_model_override(model_override: Option<String>) -> Option<String> {
    model_override
        .map(|model| model.trim().to_string())
        .filter(|model| !model.is_empty())
}

pub(crate) fn provider_role_for_model_override(model_override: Option<&str>) -> &'static str {
    // A role alias (`hint:coding`) or a retired tier slug (`hint:coding`) picks
    // that workload's route; a concrete model id rides the chat route.
    match model_override.map(str::trim) {
        Some(value) if value.starts_with("hint:") || crate::config::is_legacy_tier_model(value) => {
            match crate::inference::provider::factory::role_for_model_tier(value) {
                role @ ("agentic" | "coding" | "summarization" | "reasoning") => role,
                _ => "chat",
            }
        }
        _ => "chat",
    }
}

/// Build the in-memory config for one web-chat turn. Concrete picker values
/// may include their provider (`ollama:model`, `huggingface:org/model`) while
/// managed catalog ids use OpenRouter's `openrouter/...` form. The selected
/// route belongs only to this clone; saved chat settings and sibling workload
/// roles remain untouched.
fn effective_session_config(
    config: &Config,
    model_override: Option<&str>,
    temperature: Option<f64>,
) -> Config {
    let mut effective = config.clone();
    if let Some(model) = model_override
        .map(str::trim)
        .filter(|model| !model.is_empty())
    {
        effective.default_model = Some(model.to_string());
    }

    // Resolve the effective model even when there was no per-turn override:
    // persisted defaults must restore their provider after a restart too.
    if let Some(model) = effective
        .default_model
        .as_deref()
        .map(str::trim)
        .filter(|model| crate::inference::provider::factory::is_raw_passthrough_model(model))
    {
        if model.starts_with("openrouter/") {
            // The managed backend accepts catalog ids verbatim. This also
            // clears a stale local/BYOK chat route from the effective clone.
            effective.chat_provider = Some("openhuman".to_string());
        } else if let Some((provider, _)) = model.split_once(':') {
            let provider = provider.trim();
            let local_route = tinyinference_local::profile::is_local_provider_string(model);
            let built_in_route = matches!(provider, "claude-code" | "claude_agent_sdk");
            let configured_cloud_provider = effective
                .cloud_providers
                .iter()
                .any(|entry| entry.slug.eq_ignore_ascii_case(provider));
            if local_route || built_in_route || configured_cloud_provider {
                // Provider strings use the same `<slug>:<model>` grammar as
                // the normal inference factory; keep the full selection so
                // model ids containing additional colons remain intact.
                effective.chat_provider = Some(model.to_string());
            }
        }
    }

    if let Some(temp) = temperature {
        effective.default_temperature = temp;
    }
    effective
}

pub(super) fn build_session_agent(
    config: &Config,
    client_id: &str,
    thread_id: &str,
    target_agent_id: &str,
    model_override: Option<String>,
    temperature: Option<f64>,
    locale: Option<&str>,
) -> Result<OpenHumanSessionHost, String> {
    let effective = effective_session_config(config, model_override.as_deref(), temperature);
    let provider_role = provider_role_for_model_override(effective.default_model.as_deref());

    log::info!(
        "[web-channel] routing chat turn to '{}' provider_role='{}' (client_id={}, thread_id={})",
        target_agent_id,
        provider_role,
        client_id,
        thread_id
    );

    let locale_directive = locale.and_then(locale_reply_directive);
    if let Some(s) = locale_directive.as_deref() {
        log::info!(
            "[web-channel] injecting locale directive client={} thread={} locale={} directive={:?}",
            client_id,
            thread_id,
            locale.unwrap_or(""),
            s
        );
    }

    let agent_result = OpenHumanSessionHost::from_config_for_agent(&effective, target_agent_id);

    agent_result
        .map(|mut agent| {
            agent.set_event_context(
                json!({"client_id": client_id, "thread_id": thread_id}).to_string(),
                "web_channel",
            );
            let short_thread = if thread_id.len() > 12 {
                &thread_id[..12]
            } else {
                thread_id
            };
            agent.set_agent_definition_name(format!("{target_agent_id}_{short_thread}"));
            // Bind the conversation's durable identity here, not after
            // checkout: everything downstream — resume, transcript binding,
            // the `_meta` this session writes — is addressed by it, and a host
            // that forgot to bind it would fall back to resuming whichever
            // transcript for this agent happened to be newest.
            agent.set_thread_id(Some(thread_id));
            agent
        })
        .map_err(|e| e.to_string())
}

pub(crate) fn locale_reply_directive(locale: &str) -> Option<String> {
    let language = match locale.trim() {
        "ar" => "Arabic",
        "bn" => "Bengali",
        "es" => "Spanish",
        "fr" => "French",
        "hi" => "Hindi",
        "id" => "Indonesian",
        "it" => "Italian",
        "pt" => "Portuguese",
        "ru" => "Russian",
        "zh-CN" | "zh" => "Simplified Chinese",
        _ => return None,
    };
    Some(format!(
        "User language: the user's interface is set to {language}. \
         Respond in {language} unless the user explicitly asks for a different language. \
         Keep proper nouns, code, and command names untranslated."
    ))
}

/// Byte offset of the first difference between two signature strings, or
/// `None` when one is a prefix of the other (then the length difference is the
/// whole story).
fn first_difference_at(prior: &str, next: &str) -> Option<usize> {
    prior
        .as_bytes()
        .iter()
        .zip(next.as_bytes())
        .position(|(a, b)| a != b)
}

/// Summarise a differing signature field without printing it.
///
/// `autonomy_signature` and `model_registry_signature` are whole JSON subtrees
/// of `Config` — `config.autonomy` carries the user's `action_dir` and other
/// filesystem paths, and the registry is long. Logging either raw would be both
/// a log bomb on the chat hot path and a needless disclosure, so the log names
/// the field and gives the reader enough to find the change in their own config:
/// the two lengths and where the strings first diverge.
fn describe_signature_change(field: &str, prior: &str, next: &str) -> String {
    match first_difference_at(prior, next) {
        Some(offset) => format!(
            "{field}: differs at byte {offset} (prior {} bytes, now {} bytes)",
            prior.len(),
            next.len()
        ),
        // No differing byte in the overlap: one is a prefix of the other, i.e.
        // something was appended to or removed from the end of the subtree.
        None => format!(
            "{field}: length changed (prior {} bytes, now {} bytes)",
            prior.len(),
            next.len()
        ),
    }
}

/// The fingerprint fields that actually differ between the thread's cached
/// entry and this turn.
///
/// The cache-miss log used to print two hand-picked fields, `target_agent_id`
/// and `provider_binding`. When those two matched — which is the common case,
/// since the target agent is hard-coded to `"orchestrator"`
/// ([`pick_target_agent_id`]) — the log reported a miss while showing nothing
/// that had missed, so the warning could not be acted on. Four of the six
/// fields were invisible. This names the ones that differ instead of guessing
/// which two are interesting.
///
/// Returns an empty vector when the fingerprints are equal, which the caller
/// treats as a bug worth saying so in the log rather than silently omitting:
/// reaching the miss arm with no differing field would mean `PartialEq` and
/// this function disagree.
pub(super) fn fingerprint_diff(
    prior: &SessionCacheFingerprint,
    next: &SessionCacheFingerprint,
) -> Vec<String> {
    let mut diff = Vec::new();
    if prior.model_override != next.model_override {
        diff.push(format!(
            "model_override: {:?} -> {:?}",
            prior.model_override, next.model_override
        ));
    }
    if prior.temperature != next.temperature {
        diff.push(format!(
            "temperature: {:?} -> {:?}",
            prior.temperature, next.temperature
        ));
    }
    if prior.target_agent_id != next.target_agent_id {
        diff.push(format!(
            "target_agent_id: {} -> {}",
            prior.target_agent_id, next.target_agent_id
        ));
    }
    if prior.provider_binding != next.provider_binding {
        diff.push(format!(
            "provider_binding: {} -> {}",
            prior.provider_binding, next.provider_binding
        ));
    }
    if prior.autonomy_signature != next.autonomy_signature {
        diff.push(describe_signature_change(
            "autonomy_signature",
            &prior.autonomy_signature,
            &next.autonomy_signature,
        ));
    }
    if prior.model_registry_signature != next.model_registry_signature {
        diff.push(describe_signature_change(
            "model_registry_signature",
            &prior.model_registry_signature,
            &next.model_registry_signature,
        ));
    }
    diff
}

pub(super) fn build_session_fingerprint(
    config: &Config,
    model_override: Option<String>,
    temperature: Option<f64>,
    target_agent_id: String,
    provider_role: &str,
) -> SessionCacheFingerprint {
    let effective = effective_session_config(config, model_override.as_deref(), temperature);
    SessionCacheFingerprint {
        model_override,
        temperature,
        provider_binding: crate::inference::provider::provider_for_role(provider_role, &effective),
        target_agent_id,
        autonomy_signature: autonomy_signature(config),
        model_registry_signature: model_registry_signature(config),
    }
}

/// How `checkout_session_agent` treats the thread's cached entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CheckoutPolicy {
    /// A user turn: reuse the cached agent only when its
    /// `SessionCacheFingerprint` matches this turn's model/temperature/agent,
    /// otherwise rebuild (and cold-boot resume) with the new settings.
    Exact,
    /// A host-authored turn: reuse whatever agent the thread has, under the
    /// settings the user's last turn chose, and hand it back with that same
    /// fingerprint. The turn has no settings of its own, and rebuilding on a
    /// mismatch would evict the warm session for nothing.
    AdoptCached,
    /// A parallel fork: never take or return the cached agent; build fresh from
    /// the thread's durable history.
    Fork,
}

/// A session agent checked out of the per-thread cache for exactly one turn.
///
/// Every turn that runs on a conversation thread — a user turn, a
/// background-delivery turn, a goal continuation — must go through the same
/// checkout so it appends to the thread's live history and its transcript.
/// A turn run on a throwaway `OpenHumanSessionHost` bound to the thread writes
/// a *second* root transcript for that thread with a newer `created`, which the
/// next cold-boot resume then prefers over the real one — dropping every turn
/// the user had with the cached session (the "20–30 days" amnesia).
pub(crate) struct CheckedOutSession {
    pub(crate) agent: OpenHumanSessionHost,
    pub(crate) fingerprint: SessionCacheFingerprint,
}

/// Take the thread's cached session agent, or build one and cold-boot resume
/// it from the thread's durable history.
///
/// The entry is *removed* from the cache for the duration of the turn so two
/// turns can never drive one agent; `checkin_session_agent` /
/// `checkin_session_agent_if_vacant` put it back.
pub(crate) async fn checkout_session_agent(
    config: &Config,
    client_id: &str,
    thread_id: &str,
    model_override: Option<String>,
    temperature: Option<f64>,
    locale: Option<&str>,
    policy: CheckoutPolicy,
) -> Result<CheckedOutSession, String> {
    let map_key = super::ops::key_for(thread_id);
    let target_agent_id = pick_target_agent_id(config);
    let effective = effective_session_config(config, model_override.as_deref(), temperature);
    let provider_role = provider_role_for_model_override(effective.default_model.as_deref());
    let fingerprint = build_session_fingerprint(
        config,
        model_override.clone(),
        temperature,
        target_agent_id.clone(),
        provider_role,
    );

    // A forked (parallel) turn never reuses or evicts the shared cached agent —
    // it always builds fresh from the history snapshot below.
    let prior = if policy == CheckoutPolicy::Fork {
        None
    } else {
        let mut sessions = super::ops::THREAD_SESSIONS.lock().await;
        sessions.remove(&map_key)
    };

    let (agent, fingerprint) = match prior {
        Some(entry)
            if entry.fingerprint == fingerprint || policy == CheckoutPolicy::AdoptCached =>
        {
            log::info!(
                "[web-channel] reusing cached session agent id={} for client={} thread={}",
                entry.fingerprint.target_agent_id,
                client_id,
                thread_id
            );
            (entry.agent, entry.fingerprint)
        }
        Some(prior_entry) => {
            // Name the field(s) that actually differ. The previous log printed
            // `target_agent_id` and `provider_binding` only, and both match on
            // the common path, so a miss was reported with no visible cause
            // (openhuman#6414).
            let changed = fingerprint_diff(&prior_entry.fingerprint, &fingerprint);
            let reason = if changed.is_empty() {
                // Unreachable via `PartialEq` — reaching the miss arm with no
                // differing field would mean the derived equality and
                // `fingerprint_diff` disagree. Say so rather than log nothing.
                "no field differs (fingerprint_diff disagrees with PartialEq)".to_string()
            } else {
                changed.join("; ")
            };
            log::info!(
                "[web-channel] cache miss — rebuilding session agent \
                 (changed: {}) for client={} thread={} id={}",
                reason,
                client_id,
                thread_id,
                target_agent_id
            );
            (
                build_session_agent(
                    config,
                    client_id,
                    thread_id,
                    &target_agent_id,
                    model_override,
                    temperature,
                    locale,
                )?,
                fingerprint,
            )
        }
        None => (
            build_session_agent(
                config,
                client_id,
                thread_id,
                &target_agent_id,
                model_override,
                temperature,
                locale,
            )?,
            fingerprint,
        ),
    };

    // Cold-boot resume needs no seeding here. `set_thread_id` binds the
    // session's durable identity and the turn resumes by it, reading the one
    // transcript this conversation has ever had — tool calls, tool results and
    // reasoning included. The old path seeded by hand from whichever root
    // transcript matched the thread and newest, and fell back to the
    // conversation log's prose pairs when that failed; the prose fallback also
    // carried no system message, so such a turn reached the provider with no
    // system prompt and no prompt-cache key at all.
    Ok(CheckedOutSession { agent, fingerprint })
}

/// Return a checked-out agent to the thread cache, replacing whatever is there.
/// The primary user-turn path: it owns the thread's `IN_FLIGHT` slot, so any
/// entry it finds was left by a turn that ran concurrently and is now stale.
pub(crate) async fn checkin_session_agent(
    thread_id: &str,
    agent: OpenHumanSessionHost,
    fingerprint: SessionCacheFingerprint,
) {
    let mut sessions = super::ops::THREAD_SESSIONS.lock().await;
    sessions.insert(
        super::ops::key_for(thread_id),
        SessionEntry { agent, fingerprint },
    );
}

/// Return a checked-out agent to the thread cache only when the slot is still
/// empty. Host-authored turns (background delivery, goal continuation) do not
/// hold `IN_FLIGHT`, so a user turn that started while they ran built its own
/// agent and cached it; that one carries the user's newer turn and must win.
/// Both turns appended to the thread's durable transcript regardless.
pub(crate) async fn checkin_session_agent_if_vacant(
    thread_id: &str,
    agent: OpenHumanSessionHost,
    fingerprint: SessionCacheFingerprint,
) -> bool {
    let mut sessions = super::ops::THREAD_SESSIONS.lock().await;
    match sessions.entry(super::ops::key_for(thread_id)) {
        std::collections::hash_map::Entry::Occupied(_) => {
            log::info!(
                "[web-channel] system turn finished after a newer turn re-cached thread={} — \
                 dropping the system turn's agent",
                thread_id
            );
            false
        }
        std::collections::hash_map::Entry::Vacant(slot) => {
            slot.insert(SessionEntry { agent, fingerprint });
            true
        }
    }
}

#[cfg(test)]
#[path = "session_checkout_tests.rs"]
mod session_checkout_tests;

#[cfg(test)]
#[path = "session_routing_tests.rs"]
mod session_routing_tests;
