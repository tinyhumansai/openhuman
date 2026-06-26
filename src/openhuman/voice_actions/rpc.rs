//! RPC handlers for voice_actions domain.

use super::engine;
use serde_json::{json, Map, Value};

pub async fn handle_recognize(p: Map<String, Value>) -> Result<Value, String> {
    let utterance = p.get("utterance").and_then(|v| v.as_str()).unwrap_or("");

    // Fast path: high-confidence pattern match for simple, direct commands.
    if let Ok(ref i) = engine::recognize_intent(utterance) {
        if i.confidence >= 0.7 {
            // Auto-dispatch Safe intents immediately.
            if i.safety == super::types::ActionSafety::Safe {
                let method = format!("openhuman.{}_{}", i.namespace, i.function);
                let params = i.params.as_object().cloned().unwrap_or_default();
                let dispatch_result =
                    crate::core::all::try_invoke_registered_rpc(&method, params).await;
                if let Some(Ok(result)) = dispatch_result {
                    engine::mark_executed(&i.id, result.clone()).ok();
                    return Ok(json!({
                        "ok": true, "intent_id": i.id, "action": i.action,
                        "namespace": i.namespace, "function": i.function,
                        "confidence": i.confidence, "safety": i.safety,
                        "status": "Executed", "result": result, "source": "pattern",
                    }));
                }
            }
            return Ok(json!({
                "ok": true, "intent_id": i.id, "action": i.action,
                "namespace": i.namespace, "function": i.function,
                "confidence": i.confidence, "safety": i.safety, "status": i.status,
                "source": "pattern",
            }));
        }
    }

    // Primary path: LLM-based intent extraction for all other utterances.
    if let Some(extracted) = try_llm_recognize(utterance).await {
        let intent = engine::store_llm_intent(
            utterance,
            &extracted.action,
            extracted.confidence,
            extracted.safety,
            extracted.params,
            &extracted.description,
        );
        return Ok(json!({
            "ok": true, "intent_id": intent.id, "action": intent.action,
            "confidence": intent.confidence, "safety": intent.safety,
            "status": intent.status, "description": extracted.description,
            "params": intent.params, "source": "llm",
        }));
    }

    // Fallback: use whatever pattern matching found (even low confidence).
    match engine::recognize_intent(utterance) {
        Ok(i) => Ok(json!({
            "ok": true, "intent_id": i.id, "action": i.action,
            "namespace": i.namespace, "function": i.function,
            "confidence": i.confidence, "safety": i.safety, "status": i.status,
            "source": "pattern_fallback",
        })),
        Err(_) => {
            Ok(json!({ "ok": false, "error": format!("no matching action for: {utterance}") }))
        }
    }
}

/// LLM-based intent extraction — the primary intelligence path.
/// Pattern matching serves as a fast-path optimization for simple commands.
async fn try_llm_recognize(utterance: &str) -> Option<super::llm_intent::ExtractedIntent> {
    use super::llm_intent;
    use crate::openhuman::config::ops::load_config_with_timeout;
    use crate::openhuman::inference::provider::create_chat_provider;
    use tracing::debug;

    let actions: Vec<(String, String, super::types::ActionSafety)> = engine::list_mappings()
        .iter()
        .map(|m| (m.namespace.clone(), m.function.clone(), m.safety.clone()))
        .collect();

    let system = llm_intent::build_intent_prompt(&actions);
    let user_msg = llm_intent::build_user_message(utterance);

    let config = load_config_with_timeout().await.ok()?;
    let (provider, model) = create_chat_provider("agentic", &config).ok()?;

    let response = provider
        .chat_with_system(Some(&system), &user_msg, &model, 0.2)
        .await
        .ok()?;

    debug!(
        response_len = response.len(),
        "[voice_actions] LLM intent response received"
    );
    llm_intent::parse_llm_response(&response)
}

pub async fn handle_confirm(p: Map<String, Value>) -> Result<Value, String> {
    let id = p.get("intent_id").and_then(|v| v.as_str()).unwrap_or("");
    match engine::confirm_intent(id) {
        Ok(i) => {
            // Actually dispatch the action through the controller registry.
            let method = format!("openhuman.{}_{}", i.namespace, i.function);
            let params = match i.params.as_object() {
                Some(obj) => obj.clone(),
                None => Map::new(),
            };
            let dispatch_result =
                crate::core::all::try_invoke_registered_rpc(&method, params).await;
            match dispatch_result {
                Some(Ok(result)) => {
                    engine::mark_executed(id, result.clone()).ok();
                    Ok(
                        json!({ "ok": true, "intent_id": i.id, "status": "Executed", "result": result }),
                    )
                }
                Some(Err(e)) => {
                    engine::mark_failed(id, &e).ok();
                    Ok(json!({ "ok": true, "intent_id": i.id, "status": "Failed", "error": e }))
                }
                None => {
                    // Method not found in registry — mark executed with dispatch info.
                    Ok(
                        json!({ "ok": true, "intent_id": i.id, "status": i.status, "dispatched_to": method }),
                    )
                }
            }
        }
        Err(e) => Ok(json!({ "ok": false, "error": e })),
    }
}

pub async fn handle_reject(p: Map<String, Value>) -> Result<Value, String> {
    let id = p.get("intent_id").and_then(|v| v.as_str()).unwrap_or("");
    match engine::reject_intent(id) {
        Ok(i) => Ok(json!({ "ok": true, "intent_id": i.id, "status": i.status })),
        Err(e) => Ok(json!({ "ok": false, "error": e })),
    }
}

pub async fn handle_get_intent(p: Map<String, Value>) -> Result<Value, String> {
    let id = p.get("intent_id").and_then(|v| v.as_str()).unwrap_or("");
    match engine::get_intent(id) {
        Ok(i) => Ok(json!({
            "ok": true, "intent_id": i.id, "utterance": i.utterance,
            "action": i.action, "status": i.status, "safety": i.safety,
            "result": i.result, "error": i.error,
        })),
        Err(e) => Ok(json!({ "ok": false, "error": e })),
    }
}

pub async fn handle_list_mappings(_p: Map<String, Value>) -> Result<Value, String> {
    let mappings: Vec<Value> = engine::list_mappings()
        .iter()
        .map(|m| {
            json!({
                "pattern": m.pattern, "namespace": m.namespace,
                "function": m.function, "safety": m.safety, "description": m.description,
            })
        })
        .collect();
    Ok(json!({ "ok": true, "mappings": mappings }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn recognize_rpc() {
        let mut p = Map::new();
        p.insert("utterance".into(), Value::String("open settings".into()));
        let r = handle_recognize(p).await.unwrap();
        assert_eq!(r["ok"], true);
        assert_eq!(r["namespace"], "config");
    }

    #[tokio::test]
    async fn recognize_unknown_rpc() {
        let mut p = Map::new();
        p.insert("utterance".into(), Value::String("xyz abc".into()));
        let r = handle_recognize(p).await.unwrap();
        assert_eq!(r["ok"], false);
    }

    #[tokio::test]
    async fn list_mappings_rpc() {
        let r = handle_list_mappings(Map::new()).await.unwrap();
        assert_eq!(r["ok"], true);
        assert!(r["mappings"].as_array().unwrap().len() >= 8);
    }
}
