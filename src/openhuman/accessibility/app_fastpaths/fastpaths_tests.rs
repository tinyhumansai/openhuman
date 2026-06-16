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
    /// What `now_playing()` reports back (None = backend can't read the track).
    now_playing: Option<(String, String)>,
}

impl Backend {
    fn new(elements: Vec<AXElement>) -> Self {
        Self {
            acts: Mutex::new(Vec::new()),
            elements,
            press_fail_on: None,
            now_playing: None,
        }
    }
    fn with_now_playing(mut self, name: &str, artist: &str) -> Self {
        self.now_playing = Some((name.to_string(), artist.to_string()));
        self
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
    async fn open_url_in_app(&self, app: &str, url: &str) -> Result<String, String> {
        self.acts
            .lock()
            .unwrap()
            .push(format!("open_url_in_app:{app}:{url}"));
        Ok("ok".into())
    }
    async fn key(&self, keys: &[String]) -> Result<String, String> {
        self.acts
            .lock()
            .unwrap()
            .push(format!("key:{}", keys.join("+")));
        Ok("ok".into())
    }
    async fn now_playing(&self) -> Option<(String, String)> {
        self.now_playing.clone()
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
async fn music_fastpath_reports_verified_track_and_artist() {
    // now_playing matches the requested artist → success names the real track.
    let backend = Backend::new(vec![song_row("Numb"), AXElement::new("AXButton", "Play")])
        .with_now_playing("Numb", "Linkin Park");
    let out = music::run("play Numb by Linkin Park", &backend).await;
    assert!(out.success, "{out:?}");
    assert!(
        out.summary.contains("Linkin Park") && out.summary.contains("Numb"),
        "summary should name the verified track+artist: {}",
        out.summary
    );
}

#[tokio::test]
async fn music_fastpath_flags_wrong_artist_honestly() {
    // The search landed on a same-titled song by a different artist. The
    // fast-path must name it AND steer the agent away from re-searching (#1/#3).
    let backend = Backend::new(vec![song_row("Numb"), AXElement::new("AXButton", "Play")])
        .with_now_playing("Numb", "Tom Odell");
    let out = music::run("play Numb by Linkin Park", &backend).await;
    let s = out.summary.to_lowercase();
    assert!(
        s.contains("tom odell"),
        "must name what actually played: {}",
        out.summary
    );
    // Actionable, anti-loop guidance.
    assert!(
        s.contains("won't surface") && s.contains("library"),
        "must steer the agent (no blind re-search): {}",
        out.summary
    );
}

#[tokio::test]
async fn music_fastpath_no_row_lists_candidates_and_warns() {
    // Song rows exist but none match the query → response lists what WAS found
    // and tells the agent not to repeat the same search (#1/#2).
    let backend = Backend::new(vec![
        AXElement::new("AXCell", "Numb - Marshmello & Khalid"),
        AXElement::new("AXCell", "Numb - Tom Odell"),
    ]);
    let out = music::run("play Zelda Theme by Koji Kondo", &backend).await;
    assert!(!out.success);
    let s = out.summary.to_lowercase();
    assert!(
        s.contains("marshmello") && s.contains("tom odell"),
        "{}",
        out.summary
    );
    assert!(
        s.contains("won't help") || s.contains("don't repeat"),
        "{}",
        out.summary
    );
}

#[tokio::test]
async fn music_fastpath_artist_row_not_claimed_as_playing() {
    // Only an artist AXButton matches (no song cell) and the backend can't read
    // a track → must NOT claim "Playing"; say it only navigated (#3/#4).
    let backend = Backend::new(vec![AXElement::new("AXButton", "LINKIN PARK")]);
    let out = music::run("play Linkin Park Numb", &backend).await;
    let s = out.summary.to_lowercase();
    assert!(
        !s.contains("playing '"),
        "must not claim playback: {}",
        out.summary
    );
    assert!(
        s.contains("artist/album") && s.contains("specific song"),
        "must explain it only navigated: {}",
        out.summary
    );
    // It pressed the artist element, flagged as non-song in the step log.
    assert!(
        out.steps
            .iter()
            .any(|a| a.contains("artist/album 'LINKIN PARK'")),
        "{:?}",
        out.steps
    );
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

// ── Browser fast-path: scripted sequence ────────────────────────────

#[tokio::test]
async fn browser_nav_to_domain_succeeds_without_model() {
    let backend = Backend::new(vec![]);
    let out = super::browser::run(
        "Brave Browser",
        "open Brave and go to example.com",
        &backend,
    )
    .await;
    assert!(out.success, "pure navigation should succeed: {out:?}");
    let acts = backend.acts();
    // One deterministic open in the named browser — no model `decide` call
    // (the scripted backend panics if `decide` is hit). Bare domain is
    // normalized to https://.
    assert_eq!(acts.len(), 1, "{acts:?}");
    assert_eq!(acts[0], "open_url_in_app:Brave Browser:https://example.com");
}

#[tokio::test]
async fn browser_youtube_search_play_navigates_then_falls_through() {
    let backend = Backend::new(vec![]);
    let out = super::browser::run(
        "Brave Browser",
        "open my brave browser, go to youtube.com and play a music video",
        &backend,
    )
    .await;
    // Play intent → navigate deterministically, then return non-success so the
    // general loop performs the single first-result click via vision_click.
    assert!(!out.success, "play must defer the final click: {out:?}");
    let acts = backend.acts();
    assert_eq!(acts.len(), 1, "{acts:?}");
    assert_eq!(
        acts[0],
        "open_url_in_app:Brave Browser:https://www.youtube.com/results?search_query=music%20video"
    );
}

#[tokio::test]
async fn browser_media_control_sends_hotkey() {
    let backend = Backend::new(vec![]);
    let out = super::browser::run("Brave Browser", "pause the video", &backend).await;
    assert!(out.success, "media control should succeed: {out:?}");
    assert_eq!(backend.acts(), vec!["key:k".to_string()]);
}

#[tokio::test]
async fn browser_command_routes_resolved_shortcut() {
    // "new tab" → a cross-platform chord resolved per browser+OS. Brave maps to
    // the Chrome family; the primary modifier is OS-dependent, so accept either.
    let backend = Backend::new(vec![]);
    let out = super::browser::run("Brave Browser", "open a new tab", &backend).await;
    assert!(out.success, "browser command should succeed: {out:?}");
    let act = &backend.acts()[0];
    assert!(
        act == "key:Cmd+t" || act == "key:Ctrl+t",
        "expected a new-tab chord, got {act}"
    );
}

#[tokio::test]
async fn try_fastpath_dispatches_browser() {
    let backend = Backend::new(vec![]);
    let out = super::try_fastpath(
        "Brave Browser",
        "open Brave and go to example.com",
        &backend,
    )
    .await;
    assert!(
        out.is_some(),
        "browser nav should be claimed by a fast-path"
    );
    assert!(out.unwrap().success);
}

// ── App-shortcut fast-path (Spotify / Apple Music / Slack) ──────────

#[tokio::test]
async fn app_shortcut_spotify_next_sends_down_arrow() {
    let backend = Backend::new(vec![]);
    let out = super::app_shortcuts::run("Spotify", "next song", &backend).await;
    assert!(out.success, "{out:?}");
    // Spotify quirk: next = Down arrow.
    assert_eq!(backend.acts(), vec!["key:down".to_string()]);
}

#[tokio::test]
async fn app_shortcut_slack_quick_switcher() {
    let backend = Backend::new(vec![]);
    let out = super::app_shortcuts::run("Slack", "jump to a conversation", &backend).await;
    assert!(out.success, "{out:?}");
    let act = &backend.acts()[0];
    assert!(act == "key:Cmd+k" || act == "key:Ctrl+k", "got {act}");
}

#[tokio::test]
async fn try_fastpath_routes_apps_and_music_still_wins_play() {
    let backend = Backend::new(vec![song_row("Numb")]);
    // Apple Music "pause" → app-shortcut fast-path (music.rs declines: no query).
    assert!(super::try_fastpath("Music", "pause", &backend)
        .await
        .is_some());
    // Apple Music "play Numb" → still claimed (by music.rs, song search).
    assert!(super::try_fastpath("Music", "play Numb", &backend)
        .await
        .is_some());
    // Spotify "skip" → app-shortcut fast-path.
    assert!(super::try_fastpath("Spotify", "skip this track", &backend)
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
