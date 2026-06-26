//! RPC handlers for guided_flows domain.

use crate::rpc::RpcOutcome;
use serde_json::{json, Map, Value};
use std::time::Duration;
use tracing::debug;

use super::engine;

pub async fn handle_list_flows(_p: Map<String, Value>) -> Result<RpcOutcome<Value>, String> {
    let flows = engine::list_flows();
    let list: Vec<Value> = flows
        .iter()
        .map(|f| {
            json!({
                "id": f.id,
                "name": f.name,
                "description": f.description,
                "version": f.version,
                "step_count": f.steps.len(),
            })
        })
        .collect();
    Ok(RpcOutcome::single_log(
        json!({ "ok": true, "flows": list }),
        "listed flows",
    ))
}

pub async fn handle_start_flow(p: Map<String, Value>) -> Result<RpcOutcome<Value>, String> {
    let flow_id = p.get("flow_id").and_then(|v| v.as_str()).unwrap_or("");
    let session_id = p
        .get("session_id")
        .and_then(|v| v.as_str())
        .map(String::from);

    match engine::start_flow(flow_id, session_id) {
        Ok(s) => Ok(RpcOutcome::single_log(
            json!({
                "ok": true,
                "session_id": s.session_id,
                "flow_id": s.flow_id,
                "current_step": s.current_step,
                "state": s.state,
            }),
            format!("started flow {flow_id}"),
        )),
        Err(e) => Ok(RpcOutcome::single_log(
            json!({ "ok": false, "error": e }),
            format!("start_flow failed: {e}"),
        )),
    }
}

pub async fn handle_submit_answer(p: Map<String, Value>) -> Result<RpcOutcome<Value>, String> {
    let session_id = p.get("session_id").and_then(|v| v.as_str()).unwrap_or("");
    let step_id = p.get("step_id").and_then(|v| v.as_str()).unwrap_or("");
    let value = p.get("value").cloned().unwrap_or(Value::Null);

    match engine::submit_answer(session_id, step_id, value) {
        Ok(s) => {
            let mut resp = json!({
                "ok": true,
                "session_id": s.session_id,
                "state": s.state,
                "current_step": s.current_step,
            });
            if let Some(rec) = &s.recommendation {
                // Enhance recommendation with LLM-generated personalized summary.
                let personalized = tokio::time::timeout(
                    Duration::from_secs(4),
                    try_llm_personalize(rec, &s.answers),
                )
                .await
                .unwrap_or(None);
                resp["recommendation"] = json!({
                    "title": rec.title,
                    "summary": personalized.as_deref().unwrap_or(&rec.summary),
                    "confidence": rec.confidence,
                    "next_actions": rec.next_actions,
                });
            }
            Ok(RpcOutcome::single_log(
                resp,
                format!("submitted answer for step {step_id}"),
            ))
        }
        Err(e) => Ok(RpcOutcome::single_log(
            json!({ "ok": false, "error": e }),
            format!("submit_answer failed: {e}"),
        )),
    }
}

/// LLM-powered personalization of flow recommendations based on user answers.
async fn try_llm_personalize(
    rec: &super::types::Recommendation,
    answers: &[super::types::StepAnswer],
) -> Option<String> {
    use crate::openhuman::config::ops::load_config_with_timeout;
    use crate::openhuman::inference::provider::create_chat_provider;

    let config = load_config_with_timeout().await.ok()?;
    let (provider, model) = create_chat_provider("agentic", &config).ok()?;

    let answers_text: String = answers
        .iter()
        .map(|a| {
            format!(
                "- {}: {}",
                a.step_id,
                serde_json::to_string(&a.value).unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let prompt = format!(
        "Based on these user preferences, write a personalized 2-3 sentence recommendation summary.\n\nRecommendation: {}\nUser answers:\n{}\nNext actions: {}\n\nPersonalized summary:",
        rec.title, answers_text, rec.next_actions.join(", ")
    );

    let system = "You are a setup assistant. Write warm, personalized recommendations that reference the user's specific choices. Be concise and actionable.";

    let text = provider
        .chat_with_system(Some(system), &prompt, &model, 0.6)
        .await
        .ok()?;

    debug!("[guided_flows] LLM personalization generated");
    Some(text.trim().to_string())
}

pub async fn handle_get_session(p: Map<String, Value>) -> Result<RpcOutcome<Value>, String> {
    let session_id = p.get("session_id").and_then(|v| v.as_str()).unwrap_or("");
    match engine::get_session(session_id) {
        Ok(s) => {
            let mut resp = json!({
                "ok": true,
                "session_id": s.session_id,
                "flow_id": s.flow_id,
                "state": s.state,
                "current_step": s.current_step,
                "answers_count": s.answers.len(),
            });
            if let Some(rec) = &s.recommendation {
                resp["recommendation"] = json!({
                    "title": rec.title,
                    "summary": rec.summary,
                    "confidence": rec.confidence,
                    "next_actions": rec.next_actions,
                });
            }
            Ok(RpcOutcome::single_log(
                resp,
                format!("fetched session {session_id}"),
            ))
        }
        Err(e) => Ok(RpcOutcome::single_log(
            json!({ "ok": false, "error": e }),
            format!("get_session failed: {e}"),
        )),
    }
}

pub async fn handle_register_flow(p: Map<String, Value>) -> Result<RpcOutcome<Value>, String> {
    let id = p.get("id").and_then(|v| v.as_str()).unwrap_or("");
    let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let description = p.get("description").and_then(|v| v.as_str()).unwrap_or("");
    let start_step = p.get("start_step").and_then(|v| v.as_str()).unwrap_or("");
    let steps_raw = p.get("steps").and_then(|v| v.as_array());

    if id.is_empty() || name.is_empty() || start_step.is_empty() {
        return Ok(RpcOutcome::single_log(
            json!({"ok": false, "error": "id, name, and start_step are required"}),
            "register_flow: missing required fields",
        ));
    }

    let steps = match steps_raw {
        Some(arr) => {
            let parsed: Result<Vec<_>, String> = arr
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    let obj = s
                        .as_object()
                        .ok_or_else(|| format!("step[{i}] is not an object"))?;
                    let id = obj
                        .get("id")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| format!("step[{i}] missing required field 'id'"))?;
                    let prompt = obj
                        .get("prompt")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| format!("step[{i}] missing required field 'prompt'"))?;
                    Ok(super::types::FlowStep {
                        id: id.into(),
                        prompt: prompt.into(),
                        answer_type: match obj
                            .get("answer_type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("text")
                        {
                            "single_choice" => super::types::AnswerType::SingleChoice,
                            "multi_choice" => super::types::AnswerType::MultiChoice,
                            "boolean" => super::types::AnswerType::Boolean,
                            "number" => super::types::AnswerType::Number,
                            _ => super::types::AnswerType::FreeText,
                        },
                        choices: obj
                            .get("choices")
                            .and_then(|v| v.as_array())
                            .map(|a| {
                                a.iter()
                                    .filter_map(|v| v.as_str().map(String::from))
                                    .collect()
                            })
                            .unwrap_or_default(),
                        validation: obj
                            .get("validation")
                            .and_then(|v| v.as_str())
                            .map(String::from),
                        branches: obj
                            .get("branches")
                            .and_then(|v| v.as_object())
                            .map(|m| {
                                m.iter()
                                    .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.into())))
                                    .collect()
                            })
                            .unwrap_or_default(),
                        next: obj.get("next").and_then(|v| v.as_str()).map(String::from),
                    })
                })
                .collect();
            match parsed {
                Ok(steps) => steps,
                Err(e) => {
                    return Ok(RpcOutcome::single_log(
                        json!({"ok": false, "error": e}),
                        format!("register_flow: invalid steps: {e}"),
                    ))
                }
            }
        }
        None => {
            return Ok(RpcOutcome::single_log(
                json!({"ok": false, "error": "steps array is required"}),
                "register_flow: missing steps",
            ))
        }
    };

    match engine::register_flow(id, name, description, start_step, steps) {
        Ok(flow_id) => Ok(RpcOutcome::single_log(
            json!({"ok": true, "flow_id": flow_id}),
            format!("registered flow {id}"),
        )),
        Err(e) => Ok(RpcOutcome::single_log(
            json!({"ok": false, "error": e}),
            format!("register_flow failed: {e}"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn list_flows_rpc_returns_ok() {
        let outcome = handle_list_flows(Map::new()).await.unwrap();
        let resp = &outcome.value;
        assert_eq!(resp["ok"], true);
        assert!(resp["flows"].as_array().unwrap().len() > 0);
    }

    #[tokio::test]
    async fn start_flow_rpc_returns_session() {
        let mut p = Map::new();
        p.insert("flow_id".into(), Value::String("onboarding_setup".into()));
        p.insert("session_id".into(), Value::String("rpc-t1".into()));
        let outcome = handle_start_flow(p).await.unwrap();
        let resp = &outcome.value;
        assert_eq!(resp["ok"], true);
        assert_eq!(resp["session_id"], "rpc-t1");
    }

    #[tokio::test]
    async fn start_flow_rpc_bad_id() {
        let mut p = Map::new();
        p.insert("flow_id".into(), Value::String("nope".into()));
        let outcome = handle_start_flow(p).await.unwrap();
        assert_eq!(outcome.value["ok"], false);
    }

    #[tokio::test]
    async fn submit_answer_rpc_advances() {
        let mut p = Map::new();
        p.insert("flow_id".into(), Value::String("onboarding_setup".into()));
        p.insert("session_id".into(), Value::String("rpc-t2".into()));
        handle_start_flow(p).await.unwrap();

        let mut p = Map::new();
        p.insert("session_id".into(), Value::String("rpc-t2".into()));
        p.insert("step_id".into(), Value::String("use_case".into()));
        p.insert(
            "value".into(),
            Value::String("Personal productivity".into()),
        );
        let outcome = handle_submit_answer(p).await.unwrap();
        let resp = &outcome.value;
        assert_eq!(resp["ok"], true);
        assert_eq!(resp["current_step"], "privacy_pref");
    }

    #[tokio::test]
    async fn get_session_rpc_works() {
        let mut p = Map::new();
        p.insert("flow_id".into(), Value::String("onboarding_setup".into()));
        p.insert("session_id".into(), Value::String("rpc-t3".into()));
        handle_start_flow(p).await.unwrap();

        let mut p = Map::new();
        p.insert("session_id".into(), Value::String("rpc-t3".into()));
        let outcome = handle_get_session(p).await.unwrap();
        let resp = &outcome.value;
        assert_eq!(resp["ok"], true);
        assert_eq!(resp["state"], "active");
    }
}
