use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpoilerMode {
    Off,
    Light,
    Full,
}

impl Default for SpoilerMode {
    fn default() -> Self {
        Self::Light
    }
}

impl SpoilerMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Light => "light",
            Self::Full => "full",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HighlightKind {
    Highlight,
    Mistake,
    Coaching,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameplayFrameInput {
    pub file_name: String,
    pub image_ref: String,
    #[serde(default)]
    pub captured_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameplayPresetInput {
    pub game_id: String,
    pub display_name: String,
    #[serde(default)]
    pub coaching_focus: Vec<String>,
    #[serde(default)]
    pub audio_feedback: bool,
    #[serde(default)]
    pub spoiler_mode: SpoilerMode,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameplayReviewSessionInput {
    pub game_id: String,
    pub session_title: String,
    #[serde(default)]
    pub source_label: Option<String>,
    #[serde(default)]
    pub spoiler_mode: Option<SpoilerMode>,
    #[serde(default)]
    pub preset_id: Option<String>,
    pub frames: Vec<GameplayFrameInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameplayReviewQuestionInput {
    pub session_id: String,
    pub question: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameplayReviewPreset {
    pub game_id: String,
    pub display_name: String,
    #[serde(default)]
    pub coaching_focus: Vec<String>,
    #[serde(default)]
    pub audio_feedback: bool,
    #[serde(default)]
    pub spoiler_mode: SpoilerMode,
    #[serde(default)]
    pub notes: Option<String>,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameplayHighlight {
    pub id: String,
    pub frame_index: usize,
    #[serde(default)]
    pub captured_at_ms: Option<i64>,
    pub title: String,
    pub rationale: String,
    pub confidence: f32,
    pub kind: HighlightKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameplayClipCandidate {
    pub id: String,
    pub frame_index: usize,
    pub start_label: String,
    pub end_label: String,
    pub rationale: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameplayPlatformDraft {
    pub platform: String,
    pub title: String,
    pub description: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameplayReviewAnalysis {
    pub recap: String,
    pub highlights: Vec<GameplayHighlight>,
    pub clip_candidates: Vec<GameplayClipCandidate>,
    pub draft_metadata: Vec<GameplayPlatformDraft>,
    pub follow_up_questions: Vec<String>,
    #[serde(default)]
    pub spoiler_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameplayReviewSession {
    pub session_id: String,
    pub game_id: String,
    pub session_title: String,
    #[serde(default)]
    pub source_label: Option<String>,
    pub spoiler_mode: SpoilerMode,
    #[serde(default)]
    pub preset_id: Option<String>,
    pub imported_at_ms: i64,
    #[serde(default)]
    pub analyzed_at_ms: Option<i64>,
    pub frames: Vec<GameplayFrameInput>,
    #[serde(default)]
    pub analysis: Option<GameplayReviewAnalysis>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameplayReviewAnalysisInput {
    pub session_id: String,
    #[serde(default)]
    pub max_highlights: Option<usize>,
    #[serde(default)]
    pub platforms: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameplayReviewClipInput {
    pub session_id: String,
    #[serde(default)]
    pub platform: Option<String>,
    #[serde(default)]
    pub highlight_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameplayReviewQuestionResult {
    pub answer: String,
    pub matched_highlights: Vec<GameplayHighlight>,
    pub suggested_follow_up: Vec<String>,
}
