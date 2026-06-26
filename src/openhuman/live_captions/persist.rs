//! Optional file-based persistence for live captions transcripts.
//!
//! Saves completed transcripts as JSON files in the configured data directory.
//! On startup, previously saved transcripts can be loaded back into the store.
//!
//! ## Storage layout
//!
//! ```text
//! $DATA_DIR/live_captions/
//!   ├── lc-abc123.json
//!   ├── lc-def456.json
//!   └── ...
//! ```
//!
//! ## Log prefix
//!
//! `[live-captions-persist]`

use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

use super::types::Transcript;

const LOG_PREFIX: &str = "[live-captions-persist]";
const SUBDIR: &str = "live_captions";

/// Resolve the persistence directory from config or default.
pub fn storage_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(SUBDIR)
}

fn validate_transcript_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!("invalid transcript id: {id}"));
    }
    Ok(())
}

/// Save a transcript to disk as JSON.
pub fn save_transcript(data_dir: &Path, transcript: &Transcript) -> Result<(), String> {
    validate_transcript_id(&transcript.id)?;
    let dir = storage_dir(data_dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{LOG_PREFIX} create dir failed: {e}"))?;

    let path = dir.join(format!("{}.json", transcript.id));
    let json = serde_json::to_string_pretty(transcript)
        .map_err(|e| format!("{LOG_PREFIX} serialize failed: {e}"))?;

    std::fs::write(&path, json).map_err(|e| format!("{LOG_PREFIX} write failed: {e}"))?;

    debug!(
        "{LOG_PREFIX} saved transcript={} to {}",
        transcript.id,
        path.display()
    );
    Ok(())
}

/// Load all persisted transcripts from disk.
pub fn load_transcripts(data_dir: &Path) -> Vec<Transcript> {
    let dir = storage_dir(data_dir);
    if !dir.exists() {
        return Vec::new();
    }

    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) => {
            warn!("{LOG_PREFIX} read dir failed: {e}");
            return Vec::new();
        }
    };

    let mut transcripts = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(content) => match serde_json::from_str::<Transcript>(&content) {
                Ok(t) => {
                    debug!(
                        "{LOG_PREFIX} loaded transcript={} from {}",
                        t.id,
                        path.display()
                    );
                    transcripts.push(t);
                }
                Err(e) => warn!("{LOG_PREFIX} parse failed {}: {e}", path.display()),
            },
            Err(e) => warn!("{LOG_PREFIX} read failed {}: {e}", path.display()),
        }
    }

    info!(
        "{LOG_PREFIX} loaded {} transcripts from {}",
        transcripts.len(),
        dir.display()
    );
    transcripts
}

/// Delete a persisted transcript from disk.
pub fn delete_transcript(data_dir: &Path, transcript_id: &str) -> Result<(), String> {
    validate_transcript_id(transcript_id)?;
    let path = storage_dir(data_dir).join(format!("{transcript_id}.json"));
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| format!("{LOG_PREFIX} delete failed: {e}"))?;
        debug!("{LOG_PREFIX} deleted transcript={transcript_id}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::types::*;
    use super::*;
    use std::path::PathBuf;

    fn tmp_dir() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static CTR: AtomicU64 = AtomicU64::new(0);
        let id = CTR.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("lc_persist_test_{}_{id}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_transcript() -> Transcript {
        Transcript {
            id: "lc-test-001".into(),
            source: CaptionSource::Microphone,
            state: TranscriptState::Completed,
            title: Some("Test meeting".into()),
            segments: vec![CaptionSegment {
                text: "Hello world".into(),
                speaker: Some("Alice".into()),
                start_ms: 0,
                end_ms: 1000,
                confidence: 0.95,
                is_final: true,
            }],
            summary: Some("Test summary".into()),
            created_at: 1000,
            updated_at: 2000,
        }
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = tmp_dir();
        let t = sample_transcript();

        save_transcript(&dir, &t).unwrap();
        let loaded = load_transcripts(&dir);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, t.id);
        assert_eq!(loaded[0].segments.len(), 1);
        assert_eq!(loaded[0].segments[0].text, "Hello world");

        // Cleanup
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn delete_removes_file() {
        let dir = tmp_dir();
        let t = sample_transcript();

        save_transcript(&dir, &t).unwrap();
        delete_transcript(&dir, &t.id).unwrap();
        let loaded = load_transcripts(&dir);
        assert!(loaded.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_empty_dir() {
        let dir = tmp_dir().join("nonexistent");
        let loaded = load_transcripts(&dir);
        assert!(loaded.is_empty());
    }
}
