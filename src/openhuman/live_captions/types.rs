//! Domain types for live captions and transcript workflows.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaptionSource {
    Microphone,
    SystemAudio,
    MeetCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptionSegment {
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub speaker: Option<String>,
    pub confidence: f64,
    pub is_final: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptState {
    Recording,
    Paused,
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transcript {
    pub id: String,
    pub source: CaptionSource,
    pub state: TranscriptState,
    pub title: Option<String>,
    pub segments: Vec<CaptionSegment>,
    pub summary: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
}

impl Transcript {
    pub fn full_text(&self) -> String {
        self.segments
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn duration_ms(&self) -> u64 {
        self.segments.iter().map(|s| s.end_ms).max().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caption_source_serializes() {
        assert_eq!(
            serde_json::to_string(&CaptionSource::Microphone).unwrap(),
            "\"microphone\""
        );
        assert_eq!(
            serde_json::to_string(&CaptionSource::MeetCall).unwrap(),
            "\"meet_call\""
        );
    }

    #[test]
    fn transcript_state_serializes() {
        assert_eq!(
            serde_json::to_string(&TranscriptState::Recording).unwrap(),
            "\"recording\""
        );
        assert_eq!(
            serde_json::to_string(&TranscriptState::Completed).unwrap(),
            "\"completed\""
        );
    }

    #[test]
    fn transcript_full_text() {
        let t = Transcript {
            id: "t1".into(),
            source: CaptionSource::Microphone,
            state: TranscriptState::Completed,
            title: None,
            segments: vec![
                CaptionSegment {
                    text: "Hello".into(),
                    start_ms: 0,
                    end_ms: 500,
                    speaker: None,
                    confidence: 0.9,
                    is_final: true,
                },
                CaptionSegment {
                    text: "world".into(),
                    start_ms: 500,
                    end_ms: 1000,
                    speaker: None,
                    confidence: 0.95,
                    is_final: true,
                },
            ],
            summary: None,
            created_at: 0,
            updated_at: 0,
        };
        assert_eq!(t.full_text(), "Hello world");
        assert_eq!(t.duration_ms(), 1000);
    }

    #[test]
    fn empty_transcript_duration() {
        let t = Transcript {
            id: "t2".into(),
            source: CaptionSource::SystemAudio,
            state: TranscriptState::Recording,
            title: None,
            segments: vec![],
            summary: None,
            created_at: 0,
            updated_at: 0,
        };
        assert_eq!(t.duration_ms(), 0);
        assert_eq!(t.full_text(), "");
    }
}
