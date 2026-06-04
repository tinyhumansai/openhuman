//! Tests for the app fast-paths: pure query parsing + the Music sequence via a
//! scripted backend (no live Music, no model).

use super::super::automate::{AutomateBackend, AutomateOutcome};
use super::super::ax_interact::AXElement;
use super::music;
use async_trait::async_trait;
use std::sync::Mutex;

// ── Pure parser tests ───────────────────────────────────────────────

#[test]
fn matches_music_play_intents() {
    assert!(music::matches("Music", "play Numb by Linkin Park"));
    assert!(music::matches("Apple Music", "play Highway to Hell"));
    assert!(music::matches("music", "launch music and play Numb"));
    // Not a play intent → no fast-path.
    assert!(!music::matches("Music", "pause"));
    // Not Music → no fast-path.
    assert!(!music::matches("Slack", "play Numb"));
}

#[test]
fn extract_query_basic() {
    assert_eq!(
        music::extract_play_query("play Numb by Linkin Park").as_deref(),
        Some("Numb Linkin Park")
    );
}

#[test]
fn extract_query_strips_filler_and_suffix() {
    assert_eq!(
        music::extract_play_query("play the song Highway to Hell by AC/DC").as_deref(),
        Some("Highway to Hell AC/DC")
    );
    assert_eq!(
        music::extract_play_query("play Numb in Apple Music").as_deref(),
        Some("Numb")
    );
}

#[test]
fn extract_query_after_launch_clause() {
    assert_eq!(
        music::extract_play_query("launch Music and play Numb").as_deref(),
        Some("Numb")
    );
}

#[test]
fn extract_query_rejects_non_play() {
    assert_eq!(music::extract_play_query("pause the music"), None);
    assert_eq!(music::extract_play_query("display settings"), None); // "play" inside "display"
    assert_eq!(music::extract_play_query("play"), None); // nothing after
                                                         // Right boundary: "play" must be a whole word, not a prefix of "playback".
    assert_eq!(music::extract_play_query("open playback settings"), None);
    assert!(!music::matches("Music", "show playback options"));
}

#[test]
fn extract_query_handles_unicode_without_panicking() {
    // `to_lowercase()` can change byte lengths for non-ASCII text; the parser
    // (and replace_ci's " by " rewrite) must never slice mid-codepoint.
    assert_eq!(
        music::extract_play_query("play Café del Mar by Renée").as_deref(),
        Some("Café del Mar Renée")
    );
}

#[test]
fn extract_query_from_quoted_title_with_artist() {
    // The exact goal that failed live: song quoted earlier, sentence ends "…play it".
    assert_eq!(
        music::extract_play_query(
            "launch Music app, search for \"Highway to Hell\" by AC/DC, and play it"
        )
        .as_deref(),
        Some("Highway to Hell AC/DC")
    );
    assert_eq!(
        music::extract_play_query("play \"Numb\" by Linkin Park").as_deref(),
        Some("Numb Linkin Park")
    );
    // Quoted title, no artist.
    assert_eq!(
        music::extract_play_query("please play \"Bohemian Rhapsody\"").as_deref(),
        Some("Bohemian Rhapsody")
    );
}

#[test]
fn extract_query_rejects_bare_pronoun() {
    // No song name anywhere → decline (let the general loop / a clarifier handle it).
    assert_eq!(music::extract_play_query("play it"), None);
    assert_eq!(music::extract_play_query("play something"), None);
    assert!(!music::matches("Music", "play it"));
}

// ── Sequence test via scripted backend ──────────────────────────────

struct Backend {
    acts: Mutex<Vec<String>>,
    /// Elements returned by perceive (the search results screen).
    elements: Vec<AXElement>,
    press_fail_on: Option<String>,
}

impl Backend {
    fn new(elements: Vec<AXElement>) -> Self {
        Self {
            acts: Mutex::new(Vec::new()),
            elements,
            press_fail_on: None,
        }
    }
    fn acts(&self) -> Vec<String> {
        self.acts.lock().unwrap().clone()
    }
}

#[async_trait]
impl AutomateBackend for Backend {
    async fn perceive(&self, _app: &str, _filter: &str) -> Result<Vec<AXElement>, String> {
        Ok(self.elements.clone())
    }
    async fn decide(&self, _system: &str, _user: &str) -> Result<String, String> {
        Err("fast-path must not call the model".into())
    }
    async fn act_launch(&self, app: &str) -> Result<String, String> {
        self.acts.lock().unwrap().push(format!("launch:{app}"));
        Ok("ok".into())
    }
    async fn act_press(&self, app: &str, label: &str) -> Result<String, String> {
        self.acts
            .lock()
            .unwrap()
            .push(format!("press:{app}:{label}"));
        if self.press_fail_on.as_deref() == Some(label) {
            return Err("press failed".into());
        }
        Ok("ok".into())
    }
    async fn act_set_value(&self, _a: &str, _l: &str, _v: &str) -> Result<String, String> {
        Ok("ok".into())
    }
    async fn open_url(&self, url: &str) -> Result<String, String> {
        self.acts.lock().unwrap().push(format!("open_url:{url}"));
        Ok("ok".into())
    }
    async fn settle(&self, _app: &str) {}
    async fn wait(&self, _ms: u64) {}
}

fn song_row(label: &str) -> AXElement {
    AXElement::new("AXCell", label)
}

#[tokio::test]
async fn music_fastpath_full_sequence() {
    let backend = Backend::new(vec![song_row("Numb"), AXElement::new("AXButton", "Play")]);
    let out = music::run("play Numb by Linkin Park", &backend).await;
    assert!(out.success, "expected success: {out:?}");
    let acts = backend.acts();
    // launch → open search url → press the row → press detail Play.
    assert_eq!(acts[0], "launch:Music");
    assert!(acts[1].starts_with("open_url:music://"), "got {}", acts[1]);
    assert!(acts.contains(&"press:Music:Numb".to_string()), "{acts:?}");
    assert!(acts.contains(&"press:Music:Play".to_string()), "{acts:?}");
}

#[tokio::test]
async fn music_fastpath_no_row_fails_for_fallthrough() {
    // Search screen has nothing matching → fast-path fails (loop falls through).
    let backend = Backend::new(vec![AXElement::new("AXButton", "Some Unrelated Button")]);
    let out = music::run("play Numb", &backend).await;
    assert!(!out.success);
    assert!(out.summary.contains("no matching song"), "{}", out.summary);
}

#[tokio::test]
async fn music_fastpath_presses_row_even_if_reported_disabled() {
    // Apple Music reports pressable result rows as enabled=Some(false); the
    // fast-path must still press them (regression guard for the M5 mis-gate).
    let mut row = AXElement::new("AXCell", "Numb");
    row.enabled = Some(false);
    let backend = Backend::new(vec![row, AXElement::new("AXButton", "Play")]);
    let out = music::run("play Numb", &backend).await;
    assert!(out.success, "must press a 'disabled'-reported row: {out:?}");
    assert!(backend.acts().contains(&"press:Music:Numb".to_string()));
}

#[tokio::test]
async fn try_fastpath_dispatches_music_and_skips_others() {
    let backend = Backend::new(vec![song_row("Numb")]);
    // Non-music app → None (general loop handles it).
    assert!(super::try_fastpath("Slack", "play Numb", &backend)
        .await
        .is_none());
    // Music + play → Some.
    assert!(super::try_fastpath("Music", "play Numb", &backend)
        .await
        .is_some());
}

// Outcome type sanity: fast-paths build the same outcome the loop returns.
#[test]
fn outcome_shape() {
    let o = AutomateOutcome {
        success: true,
        summary: "x".into(),
        steps: vec![],
    };
    assert!(o.success);
}
