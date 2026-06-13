use serde::{Deserialize, Serialize};

pub const DEFAULT_COUNCIL_ID: &str = "default-council";
pub const DEFAULT_MODEL: &str = crate::openhuman::config::MODEL_REASONING_V1;
pub const DEFAULT_SHARED_REASONING: &str = "# Shared reasoning\n- Claims the council agrees on:\n- Open disagreements:\n- Evidence or constraints to preserve:\n- Judge synthesis notes:";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CouncilDefinition {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_jury_count")]
    pub jury_count: usize,
    #[serde(default = "default_debate_rounds")]
    pub debate_rounds: usize,
    #[serde(default = "default_seats")]
    pub seats: Vec<CouncilSeatDefinition>,
    #[serde(default = "default_judge")]
    pub judge: CouncilJudgeDefinition,
    #[serde(default = "default_shared_reasoning")]
    pub shared_reasoning: String,
    #[serde(default)]
    pub created_at_ms: i64,
    #[serde(default)]
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CouncilSeatDefinition {
    pub id: u64,
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default)]
    pub profile_id: String,
    pub name: String,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default)]
    pub brief: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CouncilJudgeDefinition {
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default)]
    pub profile_id: String,
    pub name: String,
    #[serde(default = "default_model")]
    pub model: String,
}

pub fn default_council(now_ms: i64) -> CouncilDefinition {
    CouncilDefinition {
        id: DEFAULT_COUNCIL_ID.to_string(),
        name: "Default council".to_string(),
        description: "Balanced analyst, builder, and skeptic jury.".to_string(),
        jury_count: default_jury_count(),
        debate_rounds: default_debate_rounds(),
        seats: default_seats(),
        judge: default_judge(),
        shared_reasoning: default_shared_reasoning(),
        created_at_ms: now_ms,
        updated_at_ms: now_ms,
    }
}

fn default_jury_count() -> usize {
    3
}

fn default_debate_rounds() -> usize {
    3
}

fn default_mode() -> String {
    "default".to_string()
}

fn default_model() -> String {
    DEFAULT_MODEL.to_string()
}

fn default_shared_reasoning() -> String {
    DEFAULT_SHARED_REASONING.to_string()
}

fn default_judge() -> CouncilJudgeDefinition {
    CouncilJudgeDefinition {
        mode: "default".to_string(),
        profile_id: String::new(),
        name: "Chief Judge".to_string(),
        model: DEFAULT_MODEL.to_string(),
    }
}

fn default_seats() -> Vec<CouncilSeatDefinition> {
    vec![
        CouncilSeatDefinition {
            id: 0,
            mode: "default".to_string(),
            profile_id: String::new(),
            name: "Analyst".to_string(),
            model: DEFAULT_MODEL.to_string(),
            brief: "Evidence, assumptions, and risk.".to_string(),
        },
        CouncilSeatDefinition {
            id: 1,
            mode: "default".to_string(),
            profile_id: String::new(),
            name: "Builder".to_string(),
            model: DEFAULT_MODEL.to_string(),
            brief: "Practical implementation path.".to_string(),
        },
        CouncilSeatDefinition {
            id: 2,
            mode: "default".to_string(),
            profile_id: String::new(),
            name: "Skeptic".to_string(),
            model: DEFAULT_MODEL.to_string(),
            brief: "Failure modes and missing context.".to_string(),
        },
    ]
}
