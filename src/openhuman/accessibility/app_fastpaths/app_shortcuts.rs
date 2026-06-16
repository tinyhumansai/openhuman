//! Keyboard-shortcut fast-path for Spotify, Apple Music, and Slack (Change 1.17).
//!
//! These desktop apps expose stable global shortcuts for the actions users ask
//! for by voice — "pause", "next song", "turn it up", "jump to a channel". For
//! those, driving the app's own shortcut is far faster and more reliable than
//! walking the AX tree: one `backend.key(chord)` and we're done. (Apple Music
//! *song search/play* still goes through `music.rs`, which types a query; this
//! module is transport/navigation control.)
//!
//! Like `browser_shortcuts.rs`, the table is keyed by `(intent, app, os)` and
//! returns key names the keyboard tool's `parse_key` understands, so the result
//! feeds straight into `AutomateBackend::key`. Sources: the official Spotify,
//! Apple Music (Mac + Windows), and Slack shortcut docs.

use super::browser_shortcuts::Os;
use super::AutomateBackend;
use super::AutomateOutcome;

/// Apps this fast-path can drive by shortcut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppFamily {
    Spotify,
    AppleMusic,
    Slack,
}

/// A control action. Media intents apply to Spotify/Apple Music; the rest to
/// Slack. [`shortcut`] returns `None` for an intent an app/OS doesn't support.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppIntent {
    // ── media transport ──
    PlayPause,
    NextTrack,
    PrevTrack,
    VolumeUp,
    VolumeDown,
    Mute,
    Shuffle,
    Repeat,
    MediaSearch,
    // ── Slack navigation ──
    QuickSwitcher,
    SlackSearch,
    NewMessage,
    ComposeDm,
    NextUnread,
    PrevUnread,
    NextChannel,
    PrevChannel,
    Threads,
    AllUnread,
    MarkRead,
}

/// Resolve the app family from the authoritative `app` arg first (so a stray
/// "music" in the goal text can't mis-route), then from an explicit app name in
/// the goal as a fallback.
fn resolve_family(app: &str, goal: &str) -> Option<AppFamily> {
    if let Some(f) = family_from_name(&app.to_lowercase()) {
        return Some(f);
    }
    let g = goal.to_lowercase();
    if g.contains("spotify") {
        Some(AppFamily::Spotify)
    } else if g.contains("slack") {
        Some(AppFamily::Slack)
    } else if g.contains("apple music") || g.contains("itunes") {
        Some(AppFamily::AppleMusic)
    } else {
        None
    }
}

fn family_from_name(n: &str) -> Option<AppFamily> {
    if n.contains("spotify") {
        Some(AppFamily::Spotify)
    } else if n.contains("slack") {
        Some(AppFamily::Slack)
    } else if n.contains("music") || n.contains("itunes") {
        // The macOS app's display name is literally "Music".
        Some(AppFamily::AppleMusic)
    } else {
        None
    }
}

fn keys(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

/// Primary modifier: ⌘ on macOS, Ctrl elsewhere.
fn primary(os: Os) -> &'static str {
    match os {
        Os::Mac => "Cmd",
        _ => "Ctrl",
    }
}

/// Resolve an intent to a key chord for the app + OS, or `None` if that app/OS
/// has no simple chord for it (the loop then falls through).
pub fn shortcut(intent: AppIntent, app: AppFamily, os: Os) -> Option<Vec<String>> {
    use AppIntent as I;
    let p = primary(os);
    let v = match (app, intent) {
        // ── Spotify (Mac & Windows columns; Linux ≈ Windows) ──
        (AppFamily::Spotify, I::PlayPause) => keys(&["space"]),
        // Spotify quirk: ↓ = next, ↑ = previous (not ←/→).
        (AppFamily::Spotify, I::NextTrack) => keys(&["down"]),
        (AppFamily::Spotify, I::PrevTrack) => keys(&["up"]),
        (AppFamily::Spotify, I::Mute) => keys(&["m"]),
        (AppFamily::Spotify, I::Shuffle) => match os {
            Os::Mac => keys(&["alt", "s"]),
            _ => keys(&["ctrl", "s"]),
        },
        (AppFamily::Spotify, I::Repeat) => match os {
            Os::Mac => keys(&["alt", "r"]),
            _ => keys(&["ctrl", "r"]),
        },
        (AppFamily::Spotify, I::MediaSearch) => keys(&[p, "k"]),
        // Spotify has no documented volume shortcut → fall through.
        (AppFamily::Spotify, I::VolumeUp | I::VolumeDown) => return None,

        // ── Apple Music ──
        (AppFamily::AppleMusic, I::PlayPause) => match os {
            Os::Mac => keys(&["space"]),
            _ => keys(&["ctrl", "space"]),
        },
        (AppFamily::AppleMusic, I::NextTrack) => match os {
            Os::Mac => keys(&["right"]),
            _ => keys(&["ctrl", "right"]),
        },
        (AppFamily::AppleMusic, I::PrevTrack) => match os {
            Os::Mac => keys(&["left"]),
            _ => keys(&["ctrl", "left"]),
        },
        (AppFamily::AppleMusic, I::VolumeUp) => match os {
            Os::Mac => keys(&["Cmd", "up"]),
            _ => keys(&["ctrl", "up"]),
        },
        (AppFamily::AppleMusic, I::VolumeDown) => match os {
            Os::Mac => keys(&["Cmd", "down"]),
            _ => keys(&["ctrl", "down"]),
        },
        // Search field: ⌘F on Mac. Windows uses a sequential access key
        // (Alt,N,F) we can't send as one chord → fall through there.
        (AppFamily::AppleMusic, I::MediaSearch) => match os {
            Os::Mac => keys(&["Cmd", "f"]),
            _ => return None,
        },
        // No simple mute/shuffle/repeat chord in Apple Music → fall through.
        (AppFamily::AppleMusic, I::Mute | I::Shuffle | I::Repeat) => return None,

        // ── Slack ──
        (AppFamily::Slack, I::QuickSwitcher) => keys(&[p, "k"]),
        (AppFamily::Slack, I::SlackSearch) => keys(&[p, "g"]),
        (AppFamily::Slack, I::NewMessage) => keys(&[p, "n"]),
        (AppFamily::Slack, I::ComposeDm) => keys(&[p, "shift", "k"]),
        // Option (mac) / Alt (win) both map to enigo's Alt key.
        (AppFamily::Slack, I::NextUnread) => keys(&["alt", "shift", "down"]),
        (AppFamily::Slack, I::PrevUnread) => keys(&["alt", "shift", "up"]),
        (AppFamily::Slack, I::NextChannel) => keys(&["alt", "down"]),
        (AppFamily::Slack, I::PrevChannel) => keys(&["alt", "up"]),
        (AppFamily::Slack, I::Threads) => keys(&[p, "shift", "t"]),
        (AppFamily::Slack, I::AllUnread) => keys(&[p, "shift", "a"]),
        (AppFamily::Slack, I::MarkRead) => keys(&["esc"]),

        // Any intent not applicable to the app.
        _ => return None,
    };
    Some(v)
}

/// Does this (app, goal) name one of these apps with a recognizable control?
pub fn matches(app: &str, goal: &str) -> bool {
    match resolve_family(app, goal) {
        Some(fam) => extract_intent(goal, fam)
            .and_then(|i| shortcut(i, fam, Os::current()))
            .is_some(),
        None => false,
    }
}

/// Run the shortcut fast-path: resolve the intent, send the chord, done.
pub async fn run(app: &str, goal: &str, backend: &dyn AutomateBackend) -> AutomateOutcome {
    use super::super::automate::progress;
    use crate::openhuman::overlay::OverlayAttentionTone;

    let mut steps: Vec<String> = Vec::new();
    let fam = match resolve_family(app, goal) {
        Some(f) => f,
        None => return fail("no recognized app", steps),
    };
    let intent = match extract_intent(goal, fam) {
        Some(i) => i,
        None => return fail("no recognized control intent", steps),
    };
    let chord = match shortcut(intent, fam, Os::current()) {
        Some(c) => c,
        None => return fail("no shortcut for this intent on this OS", steps),
    };
    let combo = chord.join("+");
    log::info!("[automate::app_shortcuts] ▶ {fam:?} {intent:?} keys={combo}");
    progress(format!("Pressing {combo}…"), OverlayAttentionTone::Accent);

    match backend.key(&chord).await {
        Ok(m) => {
            steps.push(format!("hotkey {combo}: {m}"));
            AutomateOutcome {
                success: true,
                summary: format!("Sent {combo} to {app}."),
                steps,
            }
        }
        Err(e) => {
            steps.push(format!("hotkey FAILED: {e}"));
            fail("could not send the shortcut", steps)
        }
    }
}

/// Parse a control intent from the goal, branching on app domain (media vs
/// Slack) so the same word ("search", "next") maps correctly.
fn extract_intent(goal: &str, fam: AppFamily) -> Option<AppIntent> {
    let l = goal.to_lowercase();
    match fam {
        AppFamily::Slack => parse_slack(&l),
        _ => parse_media(&l),
    }
}

fn parse_media(l: &str) -> Option<AppIntent> {
    use AppIntent as I;
    if l.contains("volume up")
        || l.contains("louder")
        || l.contains("turn it up")
        || l.contains("turn up")
        || l.contains("increase volume")
        || l.contains("raise the volume")
    {
        return Some(I::VolumeUp);
    }
    if l.contains("volume down")
        || l.contains("quieter")
        || l.contains("turn it down")
        || l.contains("turn down")
        || l.contains("decrease volume")
        || l.contains("lower the volume")
        || l.contains("lower volume")
    {
        return Some(I::VolumeDown);
    }
    if has_word(l, "mute") || has_word(l, "unmute") {
        return Some(I::Mute);
    }
    if has_word(l, "shuffle") {
        return Some(I::Shuffle);
    }
    if has_word(l, "repeat") || has_word(l, "loop") {
        return Some(I::Repeat);
    }
    if has_word(l, "next") || has_word(l, "skip") {
        return Some(I::NextTrack);
    }
    if l.contains("previous")
        || l.contains("last song")
        || l.contains("last track")
        || l.contains("go back a song")
    {
        return Some(I::PrevTrack);
    }
    if has_word(l, "pause") || has_word(l, "resume") || has_word(l, "unpause") {
        return Some(I::PlayPause);
    }
    if l.contains("continue playing") || l.contains("keep playing") {
        return Some(I::PlayPause);
    }
    // Bare "play" toggle — but NOT "play <song name>" (that's `music.rs`).
    if let Some(i) = word_index(l, "play") {
        let after = l[i + "play".len()..].trim();
        if after.is_empty()
            || matches!(
                after,
                "music"
                    | "it"
                    | "this"
                    | "the music"
                    | "the song"
                    | "song"
                    | "the track"
                    | "track"
                    | "playback"
                    | "the playback"
            )
        {
            return Some(I::PlayPause);
        }
    }
    if has_word(l, "search") || has_word(l, "find") {
        return Some(I::MediaSearch);
    }
    None
}

fn parse_slack(l: &str) -> Option<AppIntent> {
    use AppIntent as I;
    if l.contains("next unread") {
        Some(I::NextUnread)
    } else if l.contains("previous unread") || l.contains("prev unread") {
        Some(I::PrevUnread)
    } else if l.contains("next channel") || l.contains("next conversation") || l.contains("next dm")
    {
        Some(I::NextChannel)
    } else if l.contains("previous channel")
        || l.contains("prev channel")
        || l.contains("previous conversation")
    {
        Some(I::PrevChannel)
    } else if l.contains("all unread") || l.contains("unreads") {
        Some(I::AllUnread)
    } else if l.contains("thread") {
        Some(I::Threads)
    } else if l.contains("mark") && l.contains("read") {
        Some(I::MarkRead)
    } else if l.contains("new message") {
        Some(I::NewMessage)
    } else if l.contains("compose")
        || l.contains("direct message")
        || has_word(l, "dm")
        || l.contains("message someone")
        || l.contains("send a message")
    {
        Some(I::ComposeDm)
    } else if l.contains("jump to")
        || l.contains("quick switch")
        || l.contains("switch to")
        || l.contains("go to channel")
        || l.contains("open channel")
        || l.contains("open conversation")
        || l.contains("find channel")
    {
        Some(I::QuickSwitcher)
    } else if has_word(l, "search") || has_word(l, "find") {
        Some(I::SlackSearch)
    } else {
        None
    }
}

fn fail(msg: &str, steps: Vec<String>) -> AutomateOutcome {
    AutomateOutcome {
        success: false,
        summary: format!("App shortcut fast-path: {msg}"),
        steps,
    }
}

/// Whole-word membership test on an already-lowercased string.
fn has_word(haystack: &str, needle: &str) -> bool {
    word_index(haystack, needle).is_some()
}

fn word_index(haystack: &str, needle: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(rel) = haystack[from..].find(needle) {
        let idx = from + rel;
        let before_ok = idx == 0
            || !haystack[..idx]
                .chars()
                .next_back()
                .map(|c| c.is_alphanumeric())
                .unwrap_or(false);
        let after = idx + needle.len();
        let after_ok = haystack[after..]
            .chars()
            .next()
            .map(|c| !c.is_alphanumeric())
            .unwrap_or(true);
        if before_ok && after_ok {
            return Some(idx);
        }
        from = idx + needle.len();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use AppIntent as I;

    #[test]
    fn resolves_family_from_app_then_goal() {
        assert_eq!(resolve_family("Spotify", "x"), Some(AppFamily::Spotify));
        assert_eq!(resolve_family("Music", "x"), Some(AppFamily::AppleMusic));
        assert_eq!(resolve_family("Slack", "x"), Some(AppFamily::Slack));
        // From goal when app arg is unhelpful.
        assert_eq!(
            resolve_family("", "pause spotify"),
            Some(AppFamily::Spotify)
        );
        // A stray "music" in the goal with a browser app must NOT mis-route.
        assert_eq!(resolve_family("Brave Browser", "find music videos"), None);
    }

    #[test]
    fn spotify_transport_quirks() {
        let o = Os::Mac;
        assert_eq!(
            shortcut(I::PlayPause, AppFamily::Spotify, o),
            Some(keys(&["space"]))
        );
        // ↓ next / ↑ previous (the Spotify quirk).
        assert_eq!(
            shortcut(I::NextTrack, AppFamily::Spotify, o),
            Some(keys(&["down"]))
        );
        assert_eq!(
            shortcut(I::PrevTrack, AppFamily::Spotify, o),
            Some(keys(&["up"]))
        );
        assert_eq!(
            shortcut(I::Shuffle, AppFamily::Spotify, Os::Windows),
            Some(keys(&["ctrl", "s"]))
        );
        // No documented volume → None.
        assert_eq!(shortcut(I::VolumeUp, AppFamily::Spotify, o), None);
    }

    #[test]
    fn apple_music_mac_vs_windows() {
        assert_eq!(
            shortcut(I::PlayPause, AppFamily::AppleMusic, Os::Mac),
            Some(keys(&["space"]))
        );
        assert_eq!(
            shortcut(I::PlayPause, AppFamily::AppleMusic, Os::Windows),
            Some(keys(&["ctrl", "space"]))
        );
        assert_eq!(
            shortcut(I::VolumeUp, AppFamily::AppleMusic, Os::Mac),
            Some(keys(&["Cmd", "up"]))
        );
        assert_eq!(
            shortcut(I::NextTrack, AppFamily::AppleMusic, Os::Windows),
            Some(keys(&["ctrl", "right"]))
        );
        // Search: Mac ⌘F; Windows access-key sequence → None.
        assert_eq!(
            shortcut(I::MediaSearch, AppFamily::AppleMusic, Os::Mac),
            Some(keys(&["Cmd", "f"]))
        );
        assert_eq!(
            shortcut(I::MediaSearch, AppFamily::AppleMusic, Os::Windows),
            None
        );
    }

    #[test]
    fn slack_navigation() {
        assert_eq!(
            shortcut(I::QuickSwitcher, AppFamily::Slack, Os::Mac),
            Some(keys(&["Cmd", "k"]))
        );
        assert_eq!(
            shortcut(I::QuickSwitcher, AppFamily::Slack, Os::Windows),
            Some(keys(&["Ctrl", "k"]))
        );
        assert_eq!(
            shortcut(I::NextUnread, AppFamily::Slack, Os::Mac),
            Some(keys(&["alt", "shift", "down"]))
        );
        assert_eq!(
            shortcut(I::MarkRead, AppFamily::Slack, Os::Mac),
            Some(keys(&["esc"]))
        );
        // Media intent on Slack → None.
        assert_eq!(shortcut(I::Shuffle, AppFamily::Slack, Os::Mac), None);
    }

    #[test]
    fn parse_media_intents() {
        assert_eq!(parse_media("pause"), Some(I::PlayPause));
        assert_eq!(parse_media("resume the music"), Some(I::PlayPause));
        assert_eq!(parse_media("play"), Some(I::PlayPause));
        assert_eq!(parse_media("play the music"), Some(I::PlayPause));
        assert_eq!(parse_media("next song"), Some(I::NextTrack));
        assert_eq!(parse_media("skip this"), Some(I::NextTrack));
        assert_eq!(parse_media("previous track"), Some(I::PrevTrack));
        assert_eq!(parse_media("turn it up"), Some(I::VolumeUp));
        assert_eq!(parse_media("lower the volume"), Some(I::VolumeDown));
        assert_eq!(parse_media("mute"), Some(I::Mute));
        assert_eq!(parse_media("shuffle"), Some(I::Shuffle));
        // "play <song>" is NOT a transport toggle (music.rs owns that).
        assert_eq!(parse_media("play despacito"), None);
    }

    #[test]
    fn parse_slack_intents() {
        assert_eq!(parse_slack("jump to a channel"), Some(I::QuickSwitcher));
        assert_eq!(parse_slack("go to channel general"), Some(I::QuickSwitcher));
        assert_eq!(parse_slack("next unread"), Some(I::NextUnread));
        assert_eq!(parse_slack("next channel"), Some(I::NextChannel));
        assert_eq!(parse_slack("compose a message"), Some(I::ComposeDm));
        assert_eq!(parse_slack("open threads"), Some(I::Threads));
        assert_eq!(parse_slack("mark as read"), Some(I::MarkRead));
        assert_eq!(parse_slack("search"), Some(I::SlackSearch));
        assert_eq!(parse_slack("hello"), None);
    }

    #[test]
    fn matches_gating() {
        assert!(matches("Spotify", "next song"));
        assert!(matches("Music", "pause"));
        assert!(matches("Slack", "jump to a conversation"));
        // Unknown app or no intent → no match.
        assert!(!matches("Spotify", "hello there"));
        assert!(!matches("Notes", "next song"));
    }
}
