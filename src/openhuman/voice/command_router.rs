//! Phase 3 — fast command routing.
//!
//! Classifies a voice transcript (already stripped of the wake word by
//! [`super::always_on::extract_command`]) into a structured [`VoiceIntent`].
//! High-confidence intents (play / pause / open / volume …) can be executed
//! **directly** — `launch_app`, the Music fast-path, an `osascript` volume set —
//! skipping a full chat-LLM turn, which is what gets local commands under the
//! ~500 ms latency target. Anything we don't confidently recognize returns
//! [`VoiceIntent::Unknown`] and is handed to the agent (the LLM fallback), so
//! routing can only *shortcut*, never *block*.
//!
//! Pure + dependency-free so it's exhaustively unit-tested without audio/LLM.

/// A recognized fast-path voice command, or `Unknown` (→ LLM agent).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoiceIntent {
    /// "play <song/artist>" → media search + play.
    Play {
        query: String,
    },
    Pause,
    Resume,
    Next,
    Previous,
    /// "open/launch/start <app>".
    OpenApp {
        app: String,
    },
    /// "set volume to N" — absolute 0..=100.
    SetVolume {
        percent: u8,
    },
    VolumeUp,
    VolumeDown,
    Mute,
    Unmute,
    /// Not a confident fast command — defer to the agent / LLM.
    Unknown,
}

impl VoiceIntent {
    /// Stable, **non-PII** variant name for logging. Never includes the
    /// transcript-derived `query` / `app` fields (always-on mic path).
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Play { .. } => "play",
            Self::Pause => "pause",
            Self::Resume => "resume",
            Self::Next => "next",
            Self::Previous => "previous",
            Self::OpenApp { .. } => "open_app",
            Self::SetVolume { .. } => "set_volume",
            Self::VolumeUp => "volume_up",
            Self::VolumeDown => "volume_down",
            Self::Mute => "mute",
            Self::Unmute => "unmute",
            Self::Unknown => "unknown",
        }
    }
}

/// Normalize: lowercase, drop surrounding punctuation, collapse whitespace.
fn norm(s: &str) -> String {
    s.trim()
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c.is_whitespace() || c == '%' {
                c
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Strip a leading politeness/filler prefix ("please", "can you", "could you",
/// "hey") so "please pause" / "can you open slack" route the same as bare forms.
fn strip_filler(s: &str) -> &str {
    let fillers = [
        "please ",
        "can you please ",
        "can you ",
        "could you please ",
        "could you ",
        "would you ",
        "i want to ",
        "i want you to ",
        "go ahead and ",
    ];
    let mut cur = s;
    // Apply repeatedly so "please can you …" also reduces.
    loop {
        let mut matched = false;
        for f in fillers {
            if let Some(rest) = cur.strip_prefix(f) {
                cur = rest;
                matched = true;
                break;
            }
        }
        if !matched {
            return cur;
        }
    }
}

/// Classify a command transcript into a [`VoiceIntent`].
pub fn route(transcript: &str) -> VoiceIntent {
    let n = norm(transcript);
    let s = strip_filler(&n).trim();
    if s.is_empty() {
        return VoiceIntent::Unknown;
    }

    // ── Transport controls (exact-ish phrases) ──
    match s {
        "pause" | "pause music" | "pause the music" | "pause the song" | "stop" | "stop music"
        | "stop the music" => return VoiceIntent::Pause,
        "resume" | "resume music" | "continue" | "continue playing" | "unpause" => {
            return VoiceIntent::Resume
        }
        "next" | "next song" | "next track" | "skip" | "skip song" | "skip this" => {
            return VoiceIntent::Next
        }
        "previous" | "previous song" | "previous track" | "go back a song" | "last song" => {
            return VoiceIntent::Previous
        }
        "mute" | "mute it" | "mute the volume" | "mute audio" => return VoiceIntent::Mute,
        "unmute" | "unmute it" | "unmute the volume" => return VoiceIntent::Unmute,
        "volume up" | "turn it up" | "turn up the volume" | "louder" | "turn the volume up" => {
            return VoiceIntent::VolumeUp
        }
        "volume down"
        | "turn it down"
        | "turn down the volume"
        | "quieter"
        | "turn the volume down"
        | "lower the volume" => return VoiceIntent::VolumeDown,
        _ => {}
    }

    // ── Set volume to N ──
    if let Some(p) = parse_set_volume(s) {
        return VoiceIntent::SetVolume { percent: p };
    }

    // ── play <query> ──
    if let Some(rest) = s.strip_prefix("play ") {
        let q = clean_media_query(rest);
        if !q.is_empty() && !is_pronoun(&q) {
            return VoiceIntent::Play { query: q };
        }
    }

    // ── open / launch / start <app> ──
    for verb in ["open ", "launch ", "start ", "fire up "] {
        if let Some(rest) = s.strip_prefix(verb) {
            let app = clean_app_name(rest);
            if !app.is_empty() {
                return VoiceIntent::OpenApp { app };
            }
        }
    }

    VoiceIntent::Unknown
}

/// Parse "set volume to 40", "volume 40", "set the volume to 40 percent".
fn parse_set_volume(s: &str) -> Option<u8> {
    let candidates = [
        s.strip_prefix("set volume to "),
        s.strip_prefix("set the volume to "),
        s.strip_prefix("change volume to "),
        s.strip_prefix("change the volume to "),
        s.strip_prefix("volume to "),
        s.strip_prefix("volume "),
        s.strip_prefix("set volume "),
    ];
    let rest = candidates.into_iter().flatten().next()?;
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    let v: u32 = digits.parse().ok()?;
    Some(v.min(100) as u8)
}

/// Drop trailing app-locator words ("in apple music", "on spotify") and "by"→space.
fn clean_media_query(rest: &str) -> String {
    let mut q = rest.trim().to_string();
    for suffix in [
        " in apple music",
        " on apple music",
        " in music",
        " on spotify",
        " on music",
    ] {
        if q.ends_with(suffix) {
            q.truncate(q.len() - suffix.len());
            break;
        }
    }
    for filler in ["the song ", "the track ", "song ", "track ", "me "] {
        if let Some(r) = q.strip_prefix(filler) {
            q = r.to_string();
            break;
        }
    }
    q.replace(" by ", " ").trim().to_string()
}

/// Strip "the "/"my "/"up " noise from an app name ("open up slack" → "slack").
fn clean_app_name(rest: &str) -> String {
    let mut a = rest.trim();
    for filler in ["up ", "the ", "my "] {
        if let Some(r) = a.strip_prefix(filler) {
            a = r.trim();
            break;
        }
    }
    a.to_string()
}

fn is_pronoun(q: &str) -> bool {
    // Mirror the Music fast-path's ambiguity set (app_fastpaths::music::is_pronoun)
    // so "play them" / "play a song" / "play songs" defer to the agent too.
    matches!(
        q,
        "it" | "this" | "that" | "them" | "something" | "music" | "some music" | "a song" | "songs"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_controls() {
        assert_eq!(route("pause"), VoiceIntent::Pause);
        assert_eq!(route("Pause the music."), VoiceIntent::Pause);
        assert_eq!(route("please stop"), VoiceIntent::Pause);
        assert_eq!(route("resume"), VoiceIntent::Resume);
        assert_eq!(route("next song"), VoiceIntent::Next);
        assert_eq!(route("skip"), VoiceIntent::Next);
        assert_eq!(route("previous track"), VoiceIntent::Previous);
        assert_eq!(route("mute"), VoiceIntent::Mute);
    }

    #[test]
    fn volume_controls() {
        assert_eq!(route("turn it up"), VoiceIntent::VolumeUp);
        assert_eq!(route("louder"), VoiceIntent::VolumeUp);
        assert_eq!(route("lower the volume"), VoiceIntent::VolumeDown);
        assert_eq!(
            route("set volume to 40"),
            VoiceIntent::SetVolume { percent: 40 }
        );
        assert_eq!(
            route("set the volume to 100 percent"),
            VoiceIntent::SetVolume { percent: 100 }
        );
        assert_eq!(route("volume 25"), VoiceIntent::SetVolume { percent: 25 });
        // Clamp out-of-range.
        assert_eq!(
            route("set volume to 250"),
            VoiceIntent::SetVolume { percent: 100 }
        );
    }

    #[test]
    fn play_intent() {
        assert_eq!(
            route("play Numb by Linkin Park"),
            VoiceIntent::Play {
                query: "numb linkin park".into()
            }
        );
        assert_eq!(
            route("please play the song Highway to Hell on spotify"),
            VoiceIntent::Play {
                query: "highway to hell".into()
            }
        );
        // "play" alone or pronoun → not a confident query.
        assert_eq!(route("play it"), VoiceIntent::Unknown);
        assert_eq!(route("play"), VoiceIntent::Unknown);
    }

    #[test]
    fn open_app_intent() {
        assert_eq!(
            route("open Slack"),
            VoiceIntent::OpenApp {
                app: "slack".into()
            }
        );
        assert_eq!(
            route("can you open up Slack"),
            VoiceIntent::OpenApp {
                app: "slack".into()
            }
        );
        assert_eq!(
            route("launch the calculator"),
            VoiceIntent::OpenApp {
                app: "calculator".into()
            }
        );
    }

    #[test]
    fn unknown_falls_back_to_agent() {
        assert_eq!(route("what's the weather in Tokyo"), VoiceIntent::Unknown);
        assert_eq!(
            route("message Steven on slack saying hi"),
            VoiceIntent::Unknown
        );
        assert_eq!(route(""), VoiceIntent::Unknown);
        assert_eq!(route("   "), VoiceIntent::Unknown);
    }

    #[test]
    fn ambiguous_play_defers_to_agent() {
        // Mirrors the Music fast-path: bare pronouns / generic nouns carry no
        // song, so they must defer to the agent, not take the local route.
        for q in [
            "play it",
            "play them",
            "play a song",
            "play songs",
            "play music",
        ] {
            assert_eq!(route(q), VoiceIntent::Unknown, "{q} should be Unknown");
        }
    }

    #[test]
    fn intent_kind_is_stable_and_pii_free() {
        assert_eq!(route("pause").kind(), "pause");
        assert_eq!(route("turn it up").kind(), "volume_up");
        assert_eq!(
            VoiceIntent::Play {
                query: "secret song".into()
            }
            .kind(),
            "play"
        );
        assert_eq!(VoiceIntent::Unknown.kind(), "unknown");
    }
}
