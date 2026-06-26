//! Flow engine — state machine that drives guided recommendation sessions.

use std::collections::HashMap;
use std::sync::Mutex;
use tracing::{debug, info};

use crate::openhuman::guided_flows::types::*;
use crate::openhuman::util::{now_epoch, uuid_v4};

/// Maximum concurrent flow sessions before LRU eviction.
const MAX_SESSIONS: usize = 64;

static SESSIONS: std::sync::LazyLock<Mutex<HashMap<String, FlowSession>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

static FLOWS: std::sync::LazyLock<Mutex<HashMap<String, FlowDefinition>>> =
    std::sync::LazyLock::new(|| {
        Mutex::new(HashMap::from([
            builtin_onboarding_flow(),
            builtin_tool_recommendation_flow(),
        ]))
    });

fn builtin_onboarding_flow() -> (String, FlowDefinition) {
    let flow = FlowDefinition {
        id: "onboarding_setup".into(),
        name: "OpenHuman Setup Guide".into(),
        description: "Guides new users through initial configuration choices.".into(),
        version: 1,
        start_step: "use_case".into(),
        steps: vec![
            FlowStep {
                id: "use_case".into(),
                prompt: "What will you primarily use OpenHuman for?".into(),
                answer_type: AnswerType::SingleChoice,
                choices: vec![
                    "Personal productivity".into(),
                    "Team collaboration".into(),
                    "Development assistant".into(),
                    "Meeting assistant".into(),
                ],
                validation: None,
                branches: HashMap::from([("Meeting assistant".into(), "voice_pref".into())]),
                next: Some("privacy_pref".into()),
            },
            FlowStep {
                id: "voice_pref".into(),
                prompt: "Do you want voice interaction enabled?".into(),
                answer_type: AnswerType::Boolean,
                choices: vec![],
                validation: None,
                branches: HashMap::new(),
                next: Some("privacy_pref".into()),
            },
            FlowStep {
                id: "privacy_pref".into(),
                prompt: "How should OH handle your data?".into(),
                answer_type: AnswerType::SingleChoice,
                choices: vec![
                    "Keep everything local".into(),
                    "Allow cloud when needed".into(),
                    "Prefer cloud for quality".into(),
                ],
                validation: None,
                branches: HashMap::new(),
                next: Some("model_size".into()),
            },
            FlowStep {
                id: "model_size".into(),
                prompt: "What's your hardware like?".into(),
                answer_type: AnswerType::SingleChoice,
                choices: vec![
                    "Low-end (< 8GB RAM)".into(),
                    "Mid-range (8-16GB RAM)".into(),
                    "High-end (16GB+ RAM, GPU)".into(),
                ],
                validation: None,
                branches: HashMap::new(),
                next: None,
            },
        ],
    };
    (flow.id.clone(), flow)
}

fn builtin_tool_recommendation_flow() -> (String, FlowDefinition) {
    let flow = FlowDefinition {
        id: "tool_recommendation".into(),
        name: "Tool Recommendation Quiz".into(),
        description: "Recommends productivity tools based on workflow needs.".into(),
        version: 1,
        start_step: "work_type".into(),
        steps: vec![
            FlowStep {
                id: "work_type".into(),
                prompt: "What kind of work do you do most?".into(),
                answer_type: AnswerType::SingleChoice,
                choices: vec![
                    "Writing & documentation".into(),
                    "Code & engineering".into(),
                    "Design & creative".into(),
                    "Research & analysis".into(),
                ],
                validation: None,
                branches: HashMap::new(),
                next: Some("team_size".into()),
            },
            FlowStep {
                id: "team_size".into(),
                prompt: "How many people on your team?".into(),
                answer_type: AnswerType::SingleChoice,
                choices: vec![
                    "Just me".into(),
                    "2-5 people".into(),
                    "6-20 people".into(),
                    "20+ people".into(),
                ],
                validation: None,
                branches: HashMap::new(),
                next: Some("budget".into()),
            },
            FlowStep {
                id: "budget".into(),
                prompt: "What's your monthly budget per seat?".into(),
                answer_type: AnswerType::SingleChoice,
                choices: vec![
                    "Free only".into(),
                    "Under $10/mo".into(),
                    "$10-30/mo".into(),
                    "No limit".into(),
                ],
                validation: None,
                branches: HashMap::new(),
                next: None,
            },
        ],
    };
    (flow.id.clone(), flow)
}

pub fn list_flows() -> Vec<FlowDefinition> {
    let flows: Vec<FlowDefinition> = FLOWS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .values()
        .cloned()
        .collect();
    debug!(count = flows.len(), "[guided_flows] listing flows");
    flows
}

pub fn start_flow(flow_id: &str, session_id: Option<String>) -> Result<FlowSession, String> {
    let flows = FLOWS.lock().map_err(|e| format!("lock poisoned: {e}"))?;
    let def = flows
        .get(flow_id)
        .ok_or_else(|| format!("flow not found: {flow_id}"))?;
    let sid = session_id.unwrap_or_else(|| format!("gf-{}", uuid_v4()));
    let session = FlowSession {
        session_id: sid.clone(),
        flow_id: flow_id.to_string(),
        state: FlowSessionState::Active,
        current_step: def.start_step.clone(),
        answers: Vec::new(),
        recommendation: None,
        created_at: now_epoch(),
    };
    let mut sessions = SESSIONS.lock().map_err(|e| format!("lock poisoned: {e}"))?;
    // Evict completed sessions first, then LRU if still at capacity.
    if sessions.len() >= MAX_SESSIONS {
        let completed: Vec<String> = sessions
            .iter()
            .filter(|(_, s)| s.state == FlowSessionState::Completed)
            .map(|(id, _)| id.clone())
            .collect();
        for id in completed {
            sessions.remove(&id);
        }
    }
    if sessions.len() >= MAX_SESSIONS {
        // Evict oldest by created_at.
        if let Some(oldest_id) = sessions
            .values()
            .min_by_key(|s| s.created_at)
            .map(|s| s.session_id.clone())
        {
            debug!(evicted = %oldest_id, "[guided_flows] evicting LRU session (at capacity)");
            sessions.remove(&oldest_id);
        }
    }
    sessions.insert(sid, session.clone());
    info!(flow_id = %flow_id, session_id = %session.session_id, "[guided_flows] flow started");
    Ok(session)
}

pub fn submit_answer(
    session_id: &str,
    step_id: &str,
    value: serde_json::Value,
) -> Result<FlowSession, String> {
    debug!(session_id = %session_id, step_id = %step_id, "[guided_flows] answer submitted");
    // Lock ordering: FLOWS first, then SESSIONS (matches start_flow).
    let flows = FLOWS.lock().map_err(|e| format!("lock poisoned: {e}"))?;

    let mut sessions = SESSIONS.lock().map_err(|e| format!("lock poisoned: {e}"))?;
    let session = sessions
        .get_mut(session_id)
        .ok_or_else(|| format!("session not found: {session_id}"))?;
    if session.state != FlowSessionState::Active {
        return Err("session is not active".into());
    }
    if session.current_step != step_id {
        return Err(format!(
            "expected step '{}', got '{step_id}'",
            session.current_step
        ));
    }

    let def = flows
        .get(&session.flow_id)
        .ok_or_else(|| format!("flow definition missing: {}", session.flow_id))?;
    let step = def
        .steps
        .iter()
        .find(|s| s.id == step_id)
        .ok_or_else(|| format!("step not found: {step_id}"))?;

    validate_answer(step, &value)?;
    session.answers.push(StepAnswer {
        step_id: step_id.to_string(),
        value: value.clone(),
    });

    let answer_str = value.as_str().unwrap_or("").to_string();
    let next = step
        .branches
        .get(&answer_str)
        .cloned()
        .or_else(|| step.next.clone());

    match next {
        Some(next_id) => {
            session.current_step = next_id;
        }
        None => {
            session.state = FlowSessionState::Completed;
            session.recommendation = Some(generate_recommendation(def, &session.answers));
            info!(session_id = %session_id, "[guided_flows] flow completed");
        }
    }
    Ok(session.clone())
}

pub fn get_session(session_id: &str) -> Result<FlowSession, String> {
    debug!(session_id = %session_id, "[guided_flows] state queried");
    SESSIONS
        .lock()
        .map_err(|e| format!("lock poisoned: {e}"))?
        .get(session_id)
        .cloned()
        .ok_or_else(|| format!("session not found: {session_id}"))
}

pub(crate) fn validate_answer(step: &FlowStep, value: &serde_json::Value) -> Result<(), String> {
    match step.answer_type {
        AnswerType::SingleChoice => {
            let s = value.as_str().ok_or("expected string for single_choice")?;
            if !step.choices.contains(&s.to_string()) {
                return Err(format!("invalid choice: {s}"));
            }
        }
        AnswerType::MultiChoice => {
            let arr = value.as_array().ok_or("expected array for multi_choice")?;
            for v in arr {
                let s = v.as_str().ok_or("multi_choice items must be strings")?;
                if !step.choices.contains(&s.to_string()) {
                    return Err(format!("invalid choice: {s}"));
                }
            }
        }
        AnswerType::Boolean => {
            value.as_bool().ok_or("expected boolean")?;
        }
        AnswerType::Number => {
            value.as_f64().ok_or("expected number")?;
        }
        AnswerType::FreeText => {
            let s = value.as_str().ok_or("expected string for free_text")?;
            if let Some(ref pat) = step.validation {
                let re = regex::Regex::new(pat).map_err(|e| format!("bad regex: {e}"))?;
                if !re.is_match(s) {
                    return Err(format!("answer does not match: {pat}"));
                }
            }
        }
    }
    Ok(())
}

fn generate_recommendation(_def: &FlowDefinition, answers: &[StepAnswer]) -> Recommendation {
    use crate::openhuman::guided_flows::scoring::{
        accumulate_tags, rank_items, CatalogItem, ChoiceTagMapping, TagVector,
    };

    let mut metadata = HashMap::new();
    for ans in answers {
        metadata.insert(ans.step_id.clone(), ans.value.clone());
    }

    // Build tag mappings from flow choices.
    let tag_mappings: Vec<ChoiceTagMapping> = vec![
        ChoiceTagMapping {
            choice: "Personal productivity".into(),
            tags: HashMap::from([("productivity".into(), 1.0), ("local".into(), 0.5)]),
        },
        ChoiceTagMapping {
            choice: "Team collaboration".into(),
            tags: HashMap::from([("team".into(), 1.0), ("cloud".into(), 0.7)]),
        },
        ChoiceTagMapping {
            choice: "Development assistant".into(),
            tags: HashMap::from([("developer".into(), 1.0), ("local".into(), 0.8)]),
        },
        ChoiceTagMapping {
            choice: "Meeting assistant".into(),
            tags: HashMap::from([("voice".into(), 1.0), ("meetings".into(), 0.9)]),
        },
        ChoiceTagMapping {
            choice: "Keep everything local".into(),
            tags: HashMap::from([("privacy".into(), 1.0), ("local".into(), 1.0)]),
        },
        ChoiceTagMapping {
            choice: "Allow cloud when needed".into(),
            tags: HashMap::from([("cloud".into(), 0.5), ("local".into(), 0.5)]),
        },
        ChoiceTagMapping {
            choice: "Prefer cloud for quality".into(),
            tags: HashMap::from([("cloud".into(), 1.0)]),
        },
        ChoiceTagMapping {
            choice: "Low-end (< 8GB RAM)".into(),
            tags: HashMap::from([("low_end".into(), 1.0)]),
        },
        ChoiceTagMapping {
            choice: "Mid-range (8-16GB RAM)".into(),
            tags: HashMap::from([("mid_range".into(), 1.0)]),
        },
        ChoiceTagMapping {
            choice: "High-end (16GB+ RAM, GPU)".into(),
            tags: HashMap::from([("high_end".into(), 1.0)]),
        },
    ];

    // Accumulate user profile from answers.
    let mut profile = TagVector::new();
    for ans in answers {
        if let Some(s) = ans.value.as_str() {
            accumulate_tags(&mut profile, s, &tag_mappings);
        }
    }

    // Build catalog of available configuration options.
    let catalog = vec![
        CatalogItem {
            id: "voice-first".into(),
            name: "Voice-First Setup".into(),
            description: "Optimized for voice interaction".into(),
            tags: HashMap::from([("voice".into(), 1.0), ("meetings".into(), 0.8)]),
            exclude_if: vec![],
            require_tags: vec![],
            metadata: HashMap::new(),
        },
        CatalogItem {
            id: "developer-workflow".into(),
            name: "Developer Workflow Setup".into(),
            description: "Optimized for development tasks".into(),
            tags: HashMap::from([("developer".into(), 1.0), ("local".into(), 0.7)]),
            exclude_if: vec![],
            require_tags: vec![],
            metadata: HashMap::new(),
        },
        CatalogItem {
            id: "team-collab".into(),
            name: "Team Collaboration Setup".into(),
            description: "Optimized for team workflows".into(),
            tags: HashMap::from([("team".into(), 1.0), ("cloud".into(), 0.8)]),
            exclude_if: vec![],
            require_tags: vec![],
            metadata: HashMap::new(),
        },
        CatalogItem {
            id: "personal-prod".into(),
            name: "Personal Productivity Setup".into(),
            description: "Optimized for personal use".into(),
            tags: HashMap::from([("productivity".into(), 1.0), ("local".into(), 0.6)]),
            exclude_if: vec![],
            require_tags: vec![],
            metadata: HashMap::new(),
        },
    ];

    let ranked = rank_items(&profile, &catalog, 1);

    let (title, summary, confidence) = if let Some(top) = ranked.first() {
        (
            top.item_name.clone(),
            top.explanation.clone(),
            top.normalized_score.max(0.7),
        )
    } else {
        (
            "Personal Productivity Setup".into(),
            "Default recommendation".into(),
            0.5,
        )
    };

    // Generate next actions based on profile tags.
    let mut next_actions = Vec::new();
    if profile.get("privacy").copied().unwrap_or(0.0) > 0.5
        || profile.get("local").copied().unwrap_or(0.0) > 0.5
    {
        next_actions.push("Install local Whisper model for STT".into());
        next_actions.push("Install Piper for local TTS".into());
    }
    if profile.get("high_end").copied().unwrap_or(0.0) > 0.0 {
        next_actions.push("Enable large language model for better quality".into());
    } else {
        next_actions.push("Use quantized models for your hardware tier".into());
    }
    if profile.get("voice").copied().unwrap_or(0.0) > 0.5 {
        next_actions.push("Enable voice assistant in settings".into());
    }

    Recommendation {
        title,
        summary,
        confidence,
        next_actions,
        metadata,
    }
}

pub fn register_flow(
    id: &str,
    name: &str,
    description: &str,
    start_step: &str,
    steps: Vec<FlowStep>,
) -> Result<String, String> {
    if id.is_empty() || name.is_empty() || start_step.is_empty() {
        return Err("id, name, and start_step are required".into());
    }
    if steps.is_empty() {
        return Err("at least one step is required".into());
    }
    // Validate start_step exists in steps.
    if !steps.iter().any(|s| s.id == start_step) {
        return Err(format!("start_step '{}' not found in steps", start_step));
    }
    let flow = FlowDefinition {
        id: id.into(),
        name: name.into(),
        description: description.into(),
        version: 1,
        start_step: start_step.into(),
        steps,
    };
    let flow_id = flow.id.clone();
    FLOWS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(flow_id.clone(), flow);
    info!(flow_id = %flow_id, "[guided_flows] custom flow registered");
    Ok(flow_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_flows_includes_builtin() {
        assert!(list_flows().iter().any(|f| f.id == "onboarding_setup"));
    }

    #[test]
    fn start_flow_creates_session() {
        let s = start_flow("onboarding_setup", Some("eng-t1".into())).unwrap();
        assert_eq!(s.state, FlowSessionState::Active);
        assert_eq!(s.current_step, "use_case");
    }

    #[test]
    fn start_flow_unknown_errors() {
        assert!(start_flow("nope", None).unwrap_err().contains("not found"));
    }

    #[test]
    fn submit_advances_linear() {
        let s = start_flow("onboarding_setup", Some("eng-t2".into())).unwrap();
        let s = submit_answer(
            &s.session_id,
            "use_case",
            serde_json::Value::String("Personal productivity".into()),
        )
        .unwrap();
        assert_eq!(s.current_step, "privacy_pref");
    }

    #[test]
    fn submit_follows_branch() {
        let s = start_flow("onboarding_setup", Some("eng-t3".into())).unwrap();
        let s = submit_answer(
            &s.session_id,
            "use_case",
            serde_json::Value::String("Meeting assistant".into()),
        )
        .unwrap();
        assert_eq!(s.current_step, "voice_pref");
    }

    #[test]
    fn submit_validates_choice() {
        let s = start_flow("onboarding_setup", Some("eng-t4".into())).unwrap();
        assert!(submit_answer(
            &s.session_id,
            "use_case",
            serde_json::Value::String("bad".into())
        )
        .unwrap_err()
        .contains("invalid"));
    }

    #[test]
    fn submit_wrong_step_errors() {
        let s = start_flow("onboarding_setup", Some("eng-t5".into())).unwrap();
        assert!(submit_answer(
            &s.session_id,
            "privacy_pref",
            serde_json::Value::String("x".into())
        )
        .unwrap_err()
        .contains("expected step"));
    }

    #[test]
    fn full_flow_generates_recommendation() {
        let s = start_flow("onboarding_setup", Some("eng-t6".into())).unwrap();
        let s = submit_answer(
            &s.session_id,
            "use_case",
            serde_json::Value::String("Development assistant".into()),
        )
        .unwrap();
        let s = submit_answer(
            &s.session_id,
            "privacy_pref",
            serde_json::Value::String("Keep everything local".into()),
        )
        .unwrap();
        let s = submit_answer(
            &s.session_id,
            "model_size",
            serde_json::Value::String("High-end (16GB+ RAM, GPU)".into()),
        )
        .unwrap();
        assert_eq!(s.state, FlowSessionState::Completed);
        let rec = s.recommendation.unwrap();
        assert_eq!(rec.title, "Developer Workflow Setup");
        assert!(rec.next_actions.iter().any(|a| a.contains("Whisper")));
    }

    #[test]
    fn full_flow_with_branch() {
        let s = start_flow("onboarding_setup", Some("eng-t7".into())).unwrap();
        let s = submit_answer(
            &s.session_id,
            "use_case",
            serde_json::Value::String("Meeting assistant".into()),
        )
        .unwrap();
        let s = submit_answer(&s.session_id, "voice_pref", serde_json::Value::Bool(true)).unwrap();
        let s = submit_answer(
            &s.session_id,
            "privacy_pref",
            serde_json::Value::String("Allow cloud when needed".into()),
        )
        .unwrap();
        let s = submit_answer(
            &s.session_id,
            "model_size",
            serde_json::Value::String("Mid-range (8-16GB RAM)".into()),
        )
        .unwrap();
        assert_eq!(s.state, FlowSessionState::Completed);
        assert_eq!(s.recommendation.unwrap().title, "Voice-First Setup");
    }

    #[test]
    fn get_session_works() {
        let s = start_flow("onboarding_setup", Some("eng-t8".into())).unwrap();
        assert_eq!(
            get_session(&s.session_id).unwrap().state,
            FlowSessionState::Active
        );
    }

    #[test]
    fn get_session_not_found() {
        assert!(get_session("nope").unwrap_err().contains("not found"));
    }

    #[test]
    fn completed_rejects_answers() {
        let s = start_flow("onboarding_setup", Some("eng-t9".into())).unwrap();
        let s = submit_answer(
            &s.session_id,
            "use_case",
            serde_json::Value::String("Personal productivity".into()),
        )
        .unwrap();
        let s = submit_answer(
            &s.session_id,
            "privacy_pref",
            serde_json::Value::String("Keep everything local".into()),
        )
        .unwrap();
        let s = submit_answer(
            &s.session_id,
            "model_size",
            serde_json::Value::String("Low-end (< 8GB RAM)".into()),
        )
        .unwrap();
        assert!(submit_answer(&s.session_id, "x", serde_json::Value::Null)
            .unwrap_err()
            .contains("not active"));
    }

    #[test]
    fn validate_boolean_rejects_string() {
        let step = FlowStep {
            id: "t".into(),
            prompt: "?".into(),
            answer_type: AnswerType::Boolean,
            choices: vec![],
            validation: None,
            branches: HashMap::new(),
            next: None,
        };
        assert!(validate_answer(&step, &serde_json::Value::String("y".into())).is_err());
    }

    #[test]
    fn validate_number_rejects_string() {
        let step = FlowStep {
            id: "t".into(),
            prompt: "?".into(),
            answer_type: AnswerType::Number,
            choices: vec![],
            validation: None,
            branches: HashMap::new(),
            next: None,
        };
        assert!(validate_answer(&step, &serde_json::Value::String("x".into())).is_err());
    }

    #[test]
    fn validate_free_text_regex() {
        let step = FlowStep {
            id: "t".into(),
            prompt: "?".into(),
            answer_type: AnswerType::FreeText,
            choices: vec![],
            validation: Some(r"^\d{3}$".into()),
            branches: HashMap::new(),
            next: None,
        };
        assert!(validate_answer(&step, &serde_json::Value::String("123".into())).is_ok());
        assert!(validate_answer(&step, &serde_json::Value::String("abc".into())).is_err());
    }
}
