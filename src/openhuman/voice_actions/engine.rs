//! Voice action intent mapping and execution engine.

use std::collections::HashMap;
use std::sync::Mutex;
use tracing::{debug, info, warn};

use super::types::*;
use crate::openhuman::util::now_epoch;

/// Maximum stored intents before eviction.
const MAX_INTENTS: usize = 200;

/// Multi-turn context window (last N intents per session).
const CONTEXT_WINDOW: usize = 5;

/// Context timeout: 5 minutes of inactivity resets context.
const CONTEXT_TIMEOUT_SECS: u64 = 300;

/// Maximum tracked sessions in CONTEXTS before LRU eviction.
const MAX_CONTEXTS: usize = 128;

static INTENTS: std::sync::LazyLock<Mutex<HashMap<String, VoiceIntent>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

/// Per-session multi-turn context tracking.
static CONTEXTS: std::sync::LazyLock<Mutex<HashMap<String, ActionContext>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

/// Get or create a context for a session, returning recent intent IDs.
pub fn get_context(session_id: &str) -> Vec<String> {
    let mut store = CONTEXTS.lock().unwrap_or_else(|e| e.into_inner());
    let now = now_epoch();
    // True LRU: evict oldest sessions when over limit.
    if store.len() >= MAX_CONTEXTS {
        // First pass: remove stale (timed out).
        let stale: Vec<String> = store
            .iter()
            .filter(|(_, ctx)| now.saturating_sub(ctx.last_active) > CONTEXT_TIMEOUT_SECS)
            .map(|(k, _)| k.clone())
            .collect();
        for k in &stale {
            store.remove(k);
        }
        // Second pass: if still over limit, evict oldest by last_active.
        while store.len() > MAX_CONTEXTS {
            if let Some(oldest_key) = store
                .iter()
                .min_by_key(|(_, ctx)| ctx.last_active)
                .map(|(k, _)| k.clone())
            {
                store.remove(&oldest_key);
            } else {
                break;
            }
        }
    }
    if let Some(ctx) = store.get_mut(session_id) {
        if now.saturating_sub(ctx.last_active) > CONTEXT_TIMEOUT_SECS {
            // Context expired, reset.
            ctx.intents.clear();
        }
        ctx.last_active = now;
        ctx.intents
            .iter()
            .rev()
            .take(CONTEXT_WINDOW)
            .cloned()
            .collect()
    } else {
        Vec::new()
    }
}

/// Record an intent in the session context.
pub fn record_context(session_id: &str, intent_id: &str) {
    let mut store = CONTEXTS.lock().unwrap_or_else(|e| e.into_inner());
    let now = now_epoch();
    // Enforce capacity before inserting new entries.
    if store.len() >= MAX_CONTEXTS && !store.contains_key(session_id) {
        // Evict stale first, then oldest.
        let stale: Vec<String> = store
            .iter()
            .filter(|(_, ctx)| now.saturating_sub(ctx.last_active) > CONTEXT_TIMEOUT_SECS)
            .map(|(k, _)| k.clone())
            .collect();
        for k in &stale {
            store.remove(k);
        }
        while store.len() >= MAX_CONTEXTS {
            if let Some(oldest_key) = store
                .iter()
                .min_by_key(|(_, ctx)| ctx.last_active)
                .map(|(k, _)| k.clone())
            {
                store.remove(&oldest_key);
            } else {
                break;
            }
        }
    }
    let ctx = store
        .entry(session_id.to_string())
        .or_insert_with(|| ActionContext {
            session_id: session_id.into(),
            intents: Vec::new(),
            last_active: now,
        });
    ctx.intents.push(intent_id.to_string());
    ctx.last_active = now;
    // Keep only last CONTEXT_WINDOW * 2 to avoid unbounded growth.
    if ctx.intents.len() > CONTEXT_WINDOW * 2 {
        ctx.intents.drain(..ctx.intents.len() - CONTEXT_WINDOW);
    }
}

/// Built-in action mappings (keyword → controller action).
static MAPPINGS: std::sync::LazyLock<Vec<ActionMapping>> = std::sync::LazyLock::new(|| {
    vec![
        ActionMapping {
            pattern: "open settings".into(),
            namespace: "config".into(),
            function: "get".into(),
            safety: ActionSafety::Safe,
            description: "Open the settings panel".into(),
        },
        ActionMapping {
            pattern: "search".into(),
            namespace: "memory".into(),
            function: "search".into(),
            safety: ActionSafety::Safe,
            description: "Search knowledge base".into(),
        },
        ActionMapping {
            pattern: "start voice".into(),
            namespace: "voice_assistant".into(),
            function: "start_session".into(),
            safety: ActionSafety::Safe,
            description: "Start a voice assistant session".into(),
        },
        ActionMapping {
            pattern: "stop voice".into(),
            namespace: "voice_assistant".into(),
            function: "stop_session".into(),
            safety: ActionSafety::Safe,
            description: "Stop the voice assistant session".into(),
        },
        ActionMapping {
            pattern: "create draft".into(),
            namespace: "channels".into(),
            function: "create_draft".into(),
            safety: ActionSafety::Safe,
            description: "Create a message draft".into(),
        },
        ActionMapping {
            pattern: "send message".into(),
            namespace: "channels".into(),
            function: "send".into(),
            safety: ActionSafety::RequiresConfirmation,
            description: "Send a message (requires confirmation)".into(),
        },
        ActionMapping {
            pattern: "delete".into(),
            namespace: "memory".into(),
            function: "delete".into(),
            safety: ActionSafety::Destructive,
            description: "Delete data (destructive, requires confirmation)".into(),
        },
        ActionMapping {
            pattern: "check health".into(),
            namespace: "health".into(),
            function: "check".into(),
            safety: ActionSafety::Safe,
            description: "Run health diagnostics".into(),
        },
        ActionMapping {
            pattern: "list skills".into(),
            namespace: "skills".into(),
            function: "list".into(),
            safety: ActionSafety::Safe,
            description: "List available skills".into(),
        },
        ActionMapping {
            pattern: "start flow".into(),
            namespace: "guided_flows".into(),
            function: "list_flows".into(),
            safety: ActionSafety::Safe,
            description: "List guided recommendation flows".into(),
        },
    ]
});

/// Recognize intent from an utterance using keyword matching.
pub fn recognize_intent(utterance: &str) -> Result<VoiceIntent, String> {
    debug!(
        utterance_len = utterance.len(),
        "[voice_actions] recognizing intent"
    );
    let lower = utterance.to_lowercase();
    let mut best: Option<(&ActionMapping, f64)> = None;

    for mapping in MAPPINGS.iter() {
        if lower.contains(&mapping.pattern) {
            let confidence = mapping.pattern.len() as f64 / lower.len().max(1) as f64;
            let conf = confidence.min(0.99);
            if best.as_ref().map_or(true, |(_, c)| conf > *c) {
                best = Some((mapping, conf));
            }
        }
    }

    let (mapping, confidence) =
        best.ok_or_else(|| format!("no matching action for: {utterance}"))?;

    let id = uuid_v4();
    let intent = VoiceIntent {
        id: id.clone(),
        utterance: utterance.to_string(),
        action: mapping.description.clone(),
        namespace: mapping.namespace.clone(),
        function: mapping.function.clone(),
        confidence,
        safety: mapping.safety.clone(),
        status: if mapping.safety == ActionSafety::Safe {
            IntentStatus::Confirmed
        } else {
            IntentStatus::Pending
        },
        params: extract_params(utterance, mapping),
        result: None,
        error: None,
        created_at: now_epoch(),
        context_history: Vec::new(),
    };

    INTENTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id, intent.clone());
    evict_old_intents();
    if intent.status == IntentStatus::Pending {
        warn!(action_id = %intent.id, "[voice_actions] confirmation required");
    }
    info!(action_id = %intent.id, confidence = %intent.confidence, "[voice_actions] intent matched");
    Ok(intent)
}

/// Store an LLM-extracted intent in the intent store.
/// Returns the stored intent with a generated ID and proper status.
pub fn store_llm_intent(
    utterance: &str,
    action: &str,
    confidence: f64,
    safety: ActionSafety,
    params: serde_json::Value,
    _description: &str,
) -> VoiceIntent {
    let id = uuid_v4();
    let status = if safety == ActionSafety::Safe {
        IntentStatus::Confirmed
    } else {
        IntentStatus::Pending
    };
    let intent = VoiceIntent {
        id: id.clone(),
        utterance: utterance.to_string(),
        action: action.to_string(),
        namespace: "llm".to_string(),
        function: action.to_string(),
        confidence,
        safety,
        status,
        params,
        result: None,
        error: None,
        created_at: now_epoch(),
        context_history: Vec::new(),
    };
    INTENTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id, intent.clone());
    intent
}

/// Confirm a pending intent (for actions requiring confirmation) and execute it.
pub fn confirm_intent(intent_id: &str) -> Result<VoiceIntent, String> {
    let mut store = INTENTS.lock().unwrap_or_else(|e| e.into_inner());
    let intent = store
        .get_mut(intent_id)
        .ok_or_else(|| format!("intent not found: {intent_id}"))?;
    if intent.status != IntentStatus::Pending {
        return Err(format!("intent is not pending: {:?}", intent.status));
    }
    intent.status = IntentStatus::Confirmed;
    info!(action_id = %intent_id, "[voice_actions] intent confirmed, awaiting dispatch");
    Ok(intent.clone())
}

/// Reject a pending intent.
pub fn reject_intent(intent_id: &str) -> Result<VoiceIntent, String> {
    let mut store = INTENTS.lock().unwrap_or_else(|e| e.into_inner());
    let intent = store
        .get_mut(intent_id)
        .ok_or_else(|| format!("intent not found: {intent_id}"))?;
    if intent.status != IntentStatus::Pending {
        return Err(format!("intent is not pending: {:?}", intent.status));
    }
    intent.status = IntentStatus::Rejected;
    Ok(intent.clone())
}

/// Mark intent as executed (called after controller dispatch succeeds).
pub fn mark_executed(intent_id: &str, result: serde_json::Value) -> Result<VoiceIntent, String> {
    let mut store = INTENTS.lock().unwrap_or_else(|e| e.into_inner());
    let intent = store
        .get_mut(intent_id)
        .ok_or_else(|| format!("intent not found: {intent_id}"))?;
    if intent.status != IntentStatus::Confirmed {
        return Err("intent must be confirmed before execution".into());
    }
    intent.status = IntentStatus::Executed;
    intent.result = Some(result);
    Ok(intent.clone())
}

/// Mark intent as failed.
pub fn mark_failed(intent_id: &str, error: &str) -> Result<VoiceIntent, String> {
    let mut store = INTENTS.lock().unwrap_or_else(|e| e.into_inner());
    let intent = store
        .get_mut(intent_id)
        .ok_or_else(|| format!("intent not found: {intent_id}"))?;
    intent.status = IntentStatus::Failed;
    intent.error = Some(error.to_string());
    Ok(intent.clone())
}

/// Get intent by ID.
pub fn get_intent(intent_id: &str) -> Result<VoiceIntent, String> {
    INTENTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(intent_id)
        .cloned()
        .ok_or_else(|| format!("intent not found: {intent_id}"))
}

/// List all registered action mappings.
pub fn list_mappings() -> Vec<ActionMapping> {
    MAPPINGS.clone()
}

fn evict_old_intents() {
    let mut store = INTENTS.lock().unwrap_or_else(|e| e.into_inner());
    while store.len() > MAX_INTENTS {
        // Remove oldest executed/failed/rejected intent.
        let oldest = store
            .iter()
            .filter(|(_, i)| {
                matches!(
                    i.status,
                    IntentStatus::Executed | IntentStatus::Failed | IntentStatus::Rejected
                )
            })
            .min_by_key(|(_, i)| i.created_at)
            .map(|(id, _)| id.clone());
        match oldest {
            Some(id) => {
                store.remove(&id);
            }
            None => break, // No removable intents left
        }
    }
}

fn extract_params(utterance: &str, mapping: &ActionMapping) -> serde_json::Value {
    // Extract the part after the pattern as a query parameter
    let lower = utterance.to_lowercase();
    let after = lower.split(&mapping.pattern).nth(1).unwrap_or("").trim();
    if after.is_empty() {
        serde_json::json!({})
    } else {
        serde_json::json!({ "query": after })
    }
}

fn uuid_v4() -> String {
    format!("va-{}", crate::openhuman::util::uuid_v4())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognize_open_settings() {
        let i = recognize_intent("open settings please").unwrap();
        assert_eq!(i.namespace, "config");
        assert_eq!(i.function, "get");
        assert_eq!(i.safety, ActionSafety::Safe);
        assert_eq!(i.status, IntentStatus::Confirmed); // safe = auto-confirmed
    }

    #[test]
    fn recognize_search_with_query() {
        let i = recognize_intent("search for meeting notes").unwrap();
        assert_eq!(i.namespace, "memory");
        assert_eq!(i.function, "search");
        assert_eq!(i.params["query"], "for meeting notes");
    }

    #[test]
    fn recognize_send_requires_confirmation() {
        let i = recognize_intent("send message to Alice").unwrap();
        assert_eq!(i.safety, ActionSafety::RequiresConfirmation);
        assert_eq!(i.status, IntentStatus::Pending);
    }

    #[test]
    fn recognize_delete_is_destructive() {
        let i = recognize_intent("delete old files").unwrap();
        assert_eq!(i.safety, ActionSafety::Destructive);
        assert_eq!(i.status, IntentStatus::Pending);
    }

    #[test]
    fn recognize_unknown_errors() {
        assert!(recognize_intent("fly me to the moon").is_err());
    }

    #[test]
    fn confirm_pending_intent() {
        let i = recognize_intent("send message now").unwrap();
        assert_eq!(i.status, IntentStatus::Pending);
        let i = confirm_intent(&i.id).unwrap();
        assert_eq!(i.status, IntentStatus::Confirmed);
    }

    #[test]
    fn reject_pending_intent() {
        let i = recognize_intent("delete everything").unwrap();
        let i = reject_intent(&i.id).unwrap();
        assert_eq!(i.status, IntentStatus::Rejected);
    }

    #[test]
    fn confirm_non_pending_errors() {
        let i = recognize_intent("open settings").unwrap(); // auto-confirmed
        assert!(confirm_intent(&i.id).is_err());
    }

    #[test]
    fn mark_executed_works() {
        let i = recognize_intent("check health status").unwrap();
        let i = mark_executed(&i.id, serde_json::json!({"status": "ok"})).unwrap();
        assert_eq!(i.status, IntentStatus::Executed);
        assert_eq!(i.result.unwrap()["status"], "ok");
    }

    #[test]
    fn mark_failed_works() {
        let i = recognize_intent("start voice session").unwrap();
        let i = mark_failed(&i.id, "no microphone").unwrap();
        assert_eq!(i.status, IntentStatus::Failed);
        assert_eq!(i.error.unwrap(), "no microphone");
    }

    #[test]
    fn get_intent_works() {
        let i = recognize_intent("list skills available").unwrap();
        let fetched = get_intent(&i.id).unwrap();
        assert_eq!(fetched.id, i.id);
    }

    #[test]
    fn get_intent_not_found() {
        assert!(get_intent("nope").is_err());
    }

    #[test]
    fn list_mappings_not_empty() {
        assert!(list_mappings().len() >= 8);
    }

    #[test]
    fn longer_pattern_wins() {
        // "start voice" should match over "start flow"
        let i = recognize_intent("start voice assistant").unwrap();
        assert_eq!(i.namespace, "voice_assistant");
    }

    #[test]
    fn record_context_stays_bounded() {
        // Recording more than MAX_CONTEXTS distinct sessions must not grow
        // CONTEXTS past the cap — record_context evicts before inserting.
        let prefix = format!("bounded-{}-", crate::openhuman::util::uuid_v4());
        for n in 0..(MAX_CONTEXTS * 2) {
            record_context(&format!("{prefix}{n}"), &format!("intent-{n}"));
            let len = CONTEXTS.lock().unwrap_or_else(|e| e.into_inner()).len();
            assert!(
                len <= MAX_CONTEXTS,
                "CONTEXTS grew to {len}, exceeding MAX_CONTEXTS {MAX_CONTEXTS}"
            );
        }
    }
}
