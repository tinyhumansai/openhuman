//! Apple Music fast-path: "play `<song>`".
//!
//! Encodes the sequence empirically proven in tracker §1.11: open the Music
//! search URL scheme, press the matching song row to **navigate** into it, then
//! press the detail-page **Play** (a search-result press only selects/navigates;
//! the second Play press is what actually starts playback). All steps go through
//! the injectable [`AutomateBackend`], so the whole flow is unit-testable with a
//! scripted backend — no live Music, no model.

use super::super::ax_interact::AXElement;
use super::AutomateBackend;
use super::AutomateOutcome;

const APP: &str = "Music";

/// Roles that represent an actual song/track ROW — the preferred press target,
/// because pressing one navigates into (and can play) the track.
const SONG_ROLES: &[&str] = &["AXCell", "AXRow", "ListItem"];
/// Secondary roles — artist names, album headers, static text. Pressing these
/// usually only *navigates* (to an artist/album page), so they're a last resort
/// and we flag them so the caller can report honestly.
const OTHER_ROW_ROLES: &[&str] = &["AXButton", "AXStaticText"];

fn role_in(role: &str, set: &[&str]) -> bool {
    set.iter().any(|r| role.contains(r))
}

/// A chosen press target plus whether it's a real song row (vs an artist/album
/// element that only navigates). The caller uses `is_song` to avoid claiming
/// "Playing X" when it merely opened an artist page.
#[derive(Debug, Clone, PartialEq)]
struct PickedRow {
    label: String,
    is_song: bool,
}

/// Does this (app, goal) look like an Apple Music "play X" request?
pub fn matches(app: &str, goal: &str) -> bool {
    is_music_app(app) && extract_play_query(goal).is_some()
}

/// True for the Apple Music app under its common display names.
fn is_music_app(app: &str) -> bool {
    let a = app.trim().to_lowercase();
    a == "music" || a == "apple music" || a == "itunes"
}

/// Pull the search query out of a "play …" goal, or `None` if it isn't one.
///
/// Two strategies, in order:
/// 1. **Quoted title** — the orchestrator usually quotes the song, e.g.
///    `search for "Highway to Hell" by AC/DC, and play it`. Use the first
///    quoted span, plus any `by <artist>` that immediately follows it. This is
///    robust to where "play" sits in the sentence (it was the bug: a goal
///    ending in "…and play it" made the after-"play" strategy extract "it").
/// 2. **After "play"** — `play Numb by Linkin Park`, `play the song X`, etc.
///
/// Either way: drop leading `the song`/`track` filler, a trailing
/// `in/on (apple) music`, rewrite ` by ` to a space (better catalog recall),
/// and reject bare pronouns ("it"/"this"/…) that carry no song name.
pub fn extract_play_query(goal: &str) -> Option<String> {
    // Strategy 1: first quoted title (+ trailing "by artist").
    if let Some((title, rest)) = first_quoted(goal) {
        let mut q = title.trim().to_string();
        if let Some(artist) = trailing_by_artist(rest) {
            q.push(' ');
            q.push_str(&artist);
        }
        let q = clean_query(&q);
        if !q.is_empty() && !is_pronoun(&q) {
            return Some(q);
        }
    }

    // Strategy 2: text after the last word-boundary "play".
    let lower = goal.to_lowercase();
    let idx = lower.rfind("play")?;
    let before_ok = idx == 0
        || !lower[..idx]
            .chars()
            .next_back()
            .map(|c| c.is_alphabetic())
            .unwrap_or(false);
    let after_idx = idx + "play".len();
    // Right boundary too, so "playback …" isn't parsed as a play intent.
    let after_ok = lower[after_idx..]
        .chars()
        .next()
        .map(|c| !c.is_alphabetic())
        .unwrap_or(true);
    if !(before_ok && after_ok) {
        return None;
    }
    let after = &goal[after_idx..];
    let mut q = after.trim().to_string();
    for filler in ["the song ", "the track ", "song ", "track ", "me "] {
        if q.to_lowercase().starts_with(filler) {
            q = q[filler.len()..].to_string();
            break;
        }
    }
    let q = clean_query(&q);
    if q.is_empty() || is_pronoun(&q) {
        None
    } else {
        Some(q)
    }
}

/// Pull the requested artist out of a "play X by <artist>" goal — used to
/// verify we played the *right* track. The Apple Music AX row label is
/// title-only ("Numb - Single"), so a "Numb" search can resolve to the wrong
/// artist; the artist lets us confirm via the now-playing track. `None` when no
/// "by <artist>" clause is present.
pub(crate) fn extract_artist(goal: &str) -> Option<String> {
    let lower = goal.to_lowercase();
    let p = lower.find(" by ")?;
    let after = &goal[p + " by ".len()..];
    // Cut at the first clause boundary.
    let mut end = after.len();
    for delim in [",", " and ", " then ", " in ", " on ", " from ", " for "] {
        if let Some(q) = after.to_lowercase().find(delim) {
            end = end.min(q);
        }
    }
    let artist = after[..end].trim().trim_matches('"').trim().to_string();
    if artist.is_empty() || is_pronoun(&artist) {
        None
    } else {
        Some(artist)
    }
}

/// Loose artist comparison: case-insensitive, matching if either string
/// contains the other (so "Linkin Park" matches "Linkin Park feat. …").
fn artist_matches(want: &str, got: &str) -> bool {
    let w = want.trim().to_lowercase();
    let g = got.trim().to_lowercase();
    !w.is_empty() && !g.is_empty() && (g.contains(&w) || w.contains(&g))
}

/// Strip a trailing "(in|on) [apple] music" and rewrite " by " → " ".
fn clean_query(q: &str) -> String {
    let mut q = q.trim().to_string();
    let ql = q.to_lowercase();
    for suffix in [
        " in apple music",
        " on apple music",
        " in music",
        " on music",
    ] {
        if ql.ends_with(suffix) {
            q.truncate(q.len() - suffix.len());
            break;
        }
    }
    replace_ci(&q, " by ", " ").trim().to_string()
}

/// A query that's just a pronoun / generic noun carries no song — reject it so
/// the fast-path declines and the general loop (or a clarifying reply) handles it.
fn is_pronoun(q: &str) -> bool {
    matches!(
        q.trim().to_lowercase().as_str(),
        "it" | "this" | "that" | "them" | "something" | "some music" | "music" | "a song" | "songs"
    )
}

/// Return the first single- or double-quoted span and the text after its close.
fn first_quoted(s: &str) -> Option<(String, &str)> {
    // Support straight and curly double quotes.
    let opens = ['"', '\u{201C}'];
    let closes = ['"', '\u{201D}'];
    let start = s.find(|c| opens.contains(&c))?;
    let after_open = start + s[start..].chars().next()?.len_utf8();
    let rel = s[after_open..].find(|c| closes.contains(&c))?;
    let inner = &s[after_open..after_open + rel];
    let close_end = after_open + rel + s[after_open + rel..].chars().next()?.len_utf8();
    if inner.trim().is_empty() {
        return None;
    }
    Some((inner.to_string(), &s[close_end..]))
}

/// If `rest` begins with `by <artist>`, capture the artist up to the next
/// clause boundary ("," / " and " / " then " / end).
fn trailing_by_artist(rest: &str) -> Option<String> {
    let t = rest.trim_start();
    let lower = t.to_lowercase();
    let after = lower.strip_prefix("by ")?;
    let artist_region = &t[t.len() - after.len()..];
    // Cut at the first clause boundary.
    let mut end = artist_region.len();
    for delim in [",", " and ", " then ", " in ", " on "] {
        if let Some(p) = artist_region.to_lowercase().find(delim) {
            end = end.min(p);
        }
    }
    let artist = artist_region[..end].trim().to_string();
    if artist.is_empty() {
        None
    } else {
        Some(artist)
    }
}

/// Case-insensitive replace of `needle` with `repl` in `haystack`.
fn replace_ci(haystack: &str, needle: &str, repl: &str) -> String {
    if needle.is_empty() {
        return haystack.to_string();
    }
    let nl = needle.to_lowercase();
    let mut out = String::with_capacity(haystack.len());
    let mut rest = haystack;
    while !rest.is_empty() {
        // Compare on `rest` itself (never index the lowercased copy with
        // original byte offsets — `to_lowercase` can change byte lengths for
        // Unicode, which would slice mid-codepoint and panic).
        if rest.len() >= needle.len()
            && rest.is_char_boundary(needle.len())
            && rest[..needle.len()].to_lowercase() == nl
        {
            out.push_str(repl);
            rest = &rest[needle.len()..];
        } else {
            let ch = rest.chars().next().unwrap();
            out.push(ch);
            rest = &rest[ch.len_utf8()..];
        }
    }
    out
}

/// Build the Apple Music search URL scheme for `query`.
fn search_url(query: &str) -> String {
    format!(
        "music://music.apple.com/search?term={}",
        percent_encode(query)
    )
}

/// Percent-encode the reserved characters that matter in a query value
/// (space + the URL delimiters). Enough for app URL schemes; not a full
/// RFC-3986 encoder.
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The first query token worth filtering on (length > 2 so "to"/"by" don't
/// match everything). Used as the perceive filter: the snapshot's substring
/// filter can't match a whole multi-word title, so we narrow by one strong
/// token and let `pick_row` do the full token match.
fn first_token(query: &str) -> String {
    query
        .split_whitespace()
        .find(|t| t.len() > 2)
        .unwrap_or("")
        .to_string()
}

/// Choose the best press target. Preference order: an exact-label **song row**,
/// then a token-matching **song row**, then any exact-label element, and only as
/// a last resort an artist/album element (which merely navigates). Returns the
/// label *and* whether it's a real song row, so the caller never falsely claims
/// playback after pressing an artist/album header.
///
/// (We deliberately do NOT skip elements whose reported `enabled` is false —
/// Apple Music marks pressable result rows as disabled; see AXElement::enabled.)
fn pick_row(elements: &[AXElement], query: &str) -> Option<PickedRow> {
    let ql = query.to_lowercase();
    let tokens: Vec<&str> = ql.split_whitespace().filter(|t| t.len() > 2).collect();
    let token_hit = |e: &&AXElement| {
        let l = e.label.to_lowercase();
        tokens.iter().any(|t| l.contains(t))
    };
    let song = |e: &&AXElement| role_in(&e.role, SONG_ROLES);

    // 1. Exact match on a song row.
    if let Some(e) = elements
        .iter()
        .find(|e| song(e) && e.label.to_lowercase() == ql)
    {
        return Some(PickedRow {
            label: e.label.clone(),
            is_song: true,
        });
    }
    // 2. Token match on a song row.
    if let Some(e) = elements.iter().find(|e| song(e) && token_hit(e)) {
        return Some(PickedRow {
            label: e.label.clone(),
            is_song: true,
        });
    }
    // 3. Exact match on any element.
    if let Some(e) = elements.iter().find(|e| e.label.to_lowercase() == ql) {
        return Some(PickedRow {
            label: e.label.clone(),
            is_song: role_in(&e.role, SONG_ROLES),
        });
    }
    // 4. Last resort: an artist/album element that token-matches (navigates).
    elements
        .iter()
        .find(|e| role_in(&e.role, OTHER_ROW_ROLES) && token_hit(e))
        .map(|e| PickedRow {
            label: e.label.clone(),
            is_song: false,
        })
}

/// Up to 5 distinct song-row labels currently visible — surfaced in failure
/// responses so the agent (or user) can see what *was* found and choose.
fn candidate_labels(elements: &[AXElement]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for e in elements.iter().filter(|e| role_in(&e.role, SONG_ROLES)) {
        let l = e.label.trim().to_string();
        if !l.is_empty() && !out.contains(&l) {
            out.push(l);
            if out.len() >= 5 {
                break;
            }
        }
    }
    out
}

/// Run the play fast-path. Returns a failed [`AutomateOutcome`] (not a panic)
/// whenever a step can't proceed, so the caller falls through to the general
/// loop.
pub async fn run(goal: &str, backend: &dyn AutomateBackend) -> AutomateOutcome {
    let mut steps: Vec<String> = Vec::new();
    let query = match extract_play_query(goal) {
        Some(q) => q,
        None => {
            return fail("not a play request", steps);
        }
    };
    // The artist the user asked for (if any), used to confirm we played the
    // right track — the AX row label alone can't disambiguate same-titled songs.
    let want_artist = extract_artist(goal);
    log::info!("[automate::music] ▶ play query={query:?} artist={want_artist:?}");
    use super::super::automate::progress;
    use crate::openhuman::overlay::OverlayAttentionTone;
    progress(
        format!("Searching Music for {query}…"),
        OverlayAttentionTone::Accent,
    );

    // 1. Launch Music.
    match backend.act_launch(APP).await {
        Ok(m) => steps.push(format!("launch: {m}")),
        Err(e) => steps.push(format!("launch FAILED: {e}")),
    }
    backend.settle(APP).await;

    // 2. Open the search URL.
    let url = search_url(&query);
    match backend.open_url(&url).await {
        Ok(m) => steps.push(format!("search: {m}")),
        Err(e) => {
            steps.push(format!("search url FAILED: {e}"));
            return fail("could not open Music search", steps);
        }
    }
    // 3. Find the song row and press it to navigate in. Search results render
    //    asynchronously (the §1.13 timing race), so retry across settles, and
    //    filter the snapshot by one strong token (a substring filter can't
    //    match a whole multi-word title).
    let filter = first_token(&query);
    let mut row: Option<PickedRow> = None;
    let mut last_els: Vec<AXElement> = Vec::new();
    for attempt in 0..6 {
        backend.settle(APP).await;
        let els = backend.perceive(APP, &filter).await.unwrap_or_default();
        if let Some(r) = pick_row(&els, &query) {
            last_els = els;
            row = Some(r);
            break;
        }
        last_els = els;
        // Catalog search results arrive asynchronously (~3-4s); element-count
        // settle can report "stable" while the network fetch is still pending,
        // so wait real time between attempts rather than spinning instantly.
        log::info!("[automate::music] search results not ready (attempt {attempt}), waiting");
        backend.wait(800).await;
    }
    let row = match row {
        // #1/#2: when nothing matched, list what WAS found and tell the agent
        // not to repeat the same search — so it tries the library / asks the
        // user instead of re-delegating the identical query.
        None => return fail(&no_match_message(&query, &want_artist, &last_els), steps),
        Some(r) => r,
    };
    // Baseline count of "Play" controls *before* navigating, so we can tell
    // when the song's detail-page Play has actually rendered (vs. only the
    // toolbar transport Play that's always present).
    let plays_before = count_play_buttons(backend).await;

    // #4: record what KIND of element we pressed — a song row that plays, or an
    // artist/album element that only navigates.
    let pressed_song = row.is_song;
    let kind = if pressed_song { "song" } else { "artist/album" };
    match backend.act_press(APP, &row.label).await {
        Ok(m) => steps.push(format!("open {kind} '{}': {m}", row.label)),
        Err(e) => {
            steps.push(format!("open {kind} FAILED: {e}"));
            return fail("could not open the result", steps);
        }
    }

    // 4. Wait for the detail-page Play to appear. Pressing too early hits only
    //    the toolbar transport (empty queue → silence) — the exact false-success
    //    we hit live. Poll until a new Play control shows up (or give up after a
    //    few settles and try anyway).
    for _ in 0..5 {
        backend.settle(APP).await;
        if count_play_buttons(backend).await > plays_before {
            break;
        }
    }

    // 5. Press Play, then VERIFY real playback. If it didn't start, the press
    //    landed on the wrong Play — wait and retry a couple of times. Only
    //    report success when player state is actually "playing" (or the backend
    //    can't verify, in which case it's best-effort).
    let mut verified: Option<bool> = None;
    for attempt in 0..3 {
        match backend.act_press(APP, "Play").await {
            Ok(m) => steps.push(format!("play press (attempt {attempt}): {m}")),
            Err(e) => steps.push(format!("play press FAILED: {e}")),
        }
        backend.settle(APP).await;
        match backend.verify_playing().await {
            Some(true) => {
                verified = Some(true);
                break;
            }
            Some(false) => {
                verified = Some(false);
                // Give the detail page a beat to settle, then retry.
                tokio::time::sleep(std::time::Duration::from_millis(700)).await;
            }
            None => {
                // Can't verify (non-macOS) — accept best-effort and stop.
                verified = None;
                break;
            }
        }
    }

    if matches!(verified, Some(false)) {
        steps.push("verify: player state never reached 'playing'".to_string());
        return fail("opened the song but playback didn't start", steps);
    }

    // 6. Confirm we played the RIGHT track. The AX row label carries only the
    //    title, so "Numb" can land on the wrong artist (this is the exact bug we
    //    hit live). Ask Music for the now-playing name + artist and check it
    //    against the requested artist — so we never falsely claim success on a
    //    wrong-artist match, and the summary names what's actually playing.
    let now = backend.now_playing().await;
    let candidates = candidate_labels(&last_els);
    match (&want_artist, &now) {
        // #1/#3: played, but the wrong artist — name what's actually playing,
        // list the alternatives, and tell the agent NOT to repeat the search
        // (success=true so this guidance reaches the user instead of being
        // discarded by a fall-through to the model loop).
        (Some(want), Some((name, got))) if !artist_matches(want, got) => {
            steps.push(format!(
                "verify: now playing '{name}' by '{got}' — wanted '{want}' (mismatch)"
            ));
            progress(
                format!("Playing {name} — not {want}"),
                OverlayAttentionTone::Neutral,
            );
            AutomateOutcome {
                success: true,
                summary: format!(
                    "Now playing '{name}' by '{got}', not '{query}' by '{want}'. {}Re-running this search won't surface a different result — try the user's Library, or ask them to confirm the artist.",
                    candidates_phrase(&candidates),
                ),
                steps,
            }
        }
        // #3: verified the actual track (right artist, or none requested).
        (_, Some((name, got))) => {
            steps.push(format!("verify: now playing '{name}' by '{got}' ✓"));
            progress(format!("Playing {name}"), OverlayAttentionTone::Success);
            AutomateOutcome {
                success: true,
                summary: format!("Playing '{name}' by '{got}' in Music."),
                steps,
            }
        }
        // #3/#4: can't read the track. Never claim "Playing X" — and if we only
        // pressed an artist/album element (which just navigates), say so.
        (_, None) if !pressed_song => {
            steps.push("verify: pressed an artist/album element (navigation only)".to_string());
            AutomateOutcome {
                success: true,
                summary: format!(
                    "Opened an artist/album page for '{query}' but no specific track started — pressing that only navigates. {}Open a specific song to play it.",
                    candidates_phrase(&candidates),
                ),
                steps,
            }
        }
        // Song row pressed, but the backend can't confirm the track (non-macOS).
        (_, None) => {
            let unverified = matches!(verified, None);
            steps.push(if unverified {
                "verify: playback unverified".to_string()
            } else {
                "verify: playing ✓ (track name unknown)".to_string()
            });
            if !unverified {
                progress(format!("Playing {query}"), OverlayAttentionTone::Success);
            }
            AutomateOutcome {
                success: true,
                summary: if unverified {
                    format!("Started '{query}' in Music (playback unverified).")
                } else {
                    format!("Playing '{query}' in Music.")
                },
                steps,
            }
        }
    }
}

/// "Results seen: a, b, c. " — or empty when nothing was captured. Lets failure
/// responses show the agent/user the actual candidates to choose from.
fn candidates_phrase(cands: &[String]) -> String {
    if cands.is_empty() {
        String::new()
    } else {
        format!("Results seen: {}. ", cands.join(", "))
    }
}

/// Build the "no match" response: list what was found (if anything) and steer
/// the agent away from blindly repeating the identical search.
fn no_match_message(query: &str, want_artist: &Option<String>, els: &[AXElement]) -> String {
    let cands = candidate_labels(els);
    let by = want_artist
        .as_deref()
        .map(|a| format!(" by '{a}'"))
        .unwrap_or_default();
    if cands.is_empty() {
        format!(
            "No song results found for '{query}'{by} — the search may not have loaded, or the track isn't in the catalog/library. Don't repeat this exact search; try the user's Library or ask them to confirm the title/artist."
        )
    } else {
        format!(
            "Couldn't find '{query}'{by}. Results seen: {}. Re-running this search won't help — pick the closest match, try the Library, or ask the user.",
            cands.join(", ")
        )
    }
}

/// Count "Play"-labelled controls currently visible (toolbar + any detail-page
/// Play). Used to detect when navigation has rendered the song's own Play.
async fn count_play_buttons(backend: &dyn AutomateBackend) -> usize {
    backend
        .perceive(APP, "Play")
        .await
        .map(|els| {
            els.iter()
                .filter(|e| e.label.eq_ignore_ascii_case("Play"))
                .count()
        })
        .unwrap_or(0)
}

fn fail(msg: &str, steps: Vec<String>) -> AutomateOutcome {
    AutomateOutcome {
        success: false,
        summary: format!("Music fast-path: {msg}"),
        steps,
    }
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn first_token_skips_short_words() {
        assert_eq!(first_token("Highway to Hell AC/DC"), "Highway");
        assert_eq!(first_token("Numb Linkin Park"), "Numb");
        // All-short → empty (perceive then falls back to a broad list).
        assert_eq!(first_token("a x"), "");
    }

    #[test]
    fn percent_encode_escapes_reserved() {
        assert_eq!(percent_encode("Highway to Hell"), "Highway%20to%20Hell");
        // The slash in AC/DC must be encoded (this was the live-run bug).
        assert_eq!(percent_encode("AC/DC"), "AC%2FDC");
        assert_eq!(percent_encode("rock&roll"), "rock%26roll");
    }

    #[test]
    fn search_url_is_well_formed() {
        let u = search_url("Highway to Hell AC/DC");
        assert_eq!(
            u,
            "music://music.apple.com/search?term=Highway%20to%20Hell%20AC%2FDC"
        );
    }

    #[test]
    fn extract_artist_pulls_by_clause() {
        assert_eq!(
            extract_artist("play Numb by Linkin Park").as_deref(),
            Some("Linkin Park")
        );
        // Trailing clause is cut.
        assert_eq!(
            extract_artist("play Highway to Hell by AC/DC in Apple Music").as_deref(),
            Some("AC/DC")
        );
        assert_eq!(
            extract_artist("search for \"Numb\" by Linkin Park and play it").as_deref(),
            Some("Linkin Park")
        );
        // No "by" clause → None.
        assert_eq!(extract_artist("play Numb"), None);
    }

    #[test]
    fn artist_matches_is_loose() {
        assert!(artist_matches("Linkin Park", "Linkin Park"));
        assert!(artist_matches("Linkin Park", "Linkin Park feat. Jay-Z"));
        assert!(artist_matches("ac/dc", "AC/DC"));
        assert!(!artist_matches("Linkin Park", "Tom Odell"));
    }

    #[test]
    fn pick_row_prefers_song_row_over_artist() {
        // Token match (query has extra "AC/DC" the row label lacks).
        let els = vec![
            AXElement::new("AXCell", "Highway to Hell"),
            AXElement::new("AXButton", "Play"),
        ];
        let p = pick_row(&els, "Highway to Hell AC/DC").unwrap();
        assert_eq!(p.label, "Highway to Hell");
        assert!(p.is_song);

        // An artist AXButton and a song AXCell both token-match "Linkin Park
        // Numb" — the SONG row must win (the live bug pressed the artist).
        let els2 = vec![
            AXElement::new("AXButton", "LINKIN PARK"),
            AXElement::new("AXCell", "Numb"),
        ];
        let p2 = pick_row(&els2, "Linkin Park Numb").unwrap();
        assert_eq!(p2.label, "Numb");
        assert!(p2.is_song);

        // Only an artist element is present → chosen, but flagged non-song.
        let els3 = vec![AXElement::new("AXButton", "LINKIN PARK")];
        let p3 = pick_row(&els3, "Linkin Park Numb").unwrap();
        assert_eq!(p3.label, "LINKIN PARK");
        assert!(!p3.is_song);
    }

    #[test]
    fn no_match_message_lists_candidates_and_warns() {
        let els = vec![
            AXElement::new("AXCell", "Numb - Marshmello & Khalid"),
            AXElement::new("AXCell", "Numb - Tom Odell"),
            AXElement::new("AXButton", "Play"), // not a song row → excluded
        ];
        let m = no_match_message("Numb Linkin Park", &Some("Linkin Park".into()), &els);
        assert!(m.contains("Marshmello") && m.contains("Tom Odell"), "{m}");
        assert!(m.to_lowercase().contains("won't help"), "{m}");
        // Empty results → still actionable, no fake candidate list.
        let m2 = no_match_message("Numb", &None, &[]);
        assert!(m2.to_lowercase().contains("don't repeat"), "{m2}");
    }
}

/// Live integration test — drives the real Apple Music app. Ignored by default
/// (needs macOS, the Music app, and Accessibility permission for the runner).
///
/// Run on a Mac with:
///   cargo test --lib music_fastpath_live -- --ignored --nocapture
#[cfg(all(test, target_os = "macos"))]
mod live {
    use super::run;
    use crate::openhuman::accessibility::automate::RealBackend;

    #[tokio::test]
    #[ignore = "requires macOS + Music app + Accessibility permission"]
    async fn music_fastpath_live() {
        let backend = RealBackend::new(crate::openhuman::config::Config::default());
        let out = run("play Highway to Hell by AC/DC", &backend).await;
        // Tool-level success is asserted; actual playback is best-effort
        // (Apple Music's UI is nondeterministic — tracker §1.11/§1.13).
        println!(
            "[music_fastpath_live] success={} summary={}",
            out.success, out.summary
        );
        for s in &out.steps {
            println!("  - {s}");
        }
        let state = player_state();
        println!("[music_fastpath_live] player_state={state}");
        // Now that the flow verifies playback, hold it to the real bar:
        // the song must actually be playing.
        assert!(out.success, "fast-path reported failure: {}", out.summary);
        assert_eq!(state, "playing", "Music did not actually start playing");
    }

    /// `osascript` ground-truth for whether audio is actually playing.
    fn player_state() -> String {
        std::process::Command::new("osascript")
            .args(["-e", "tell application \"Music\" to player state as string"])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_else(|| "(osascript failed)".into())
    }

    /// Empirical probe (not an assertion): open the search, dump what Music's
    /// AX tree actually exposes, and report player state before/after each
    /// candidate press. Used to design the real play sequence.
    #[tokio::test]
    #[ignore = "probe — run manually to inspect Music's AX tree"]
    async fn music_probe() {
        use crate::openhuman::accessibility::ax_interact as ax;
        let q = "Highway to Hell";
        let _ = std::process::Command::new("open")
            .arg("-a")
            .arg("Music")
            .status();
        std::thread::sleep(std::time::Duration::from_secs(3));
        let _ = std::process::Command::new("open")
            .arg(format!(
                "music://music.apple.com/search?term={}",
                q.replace(' ', "%20")
            ))
            .status();
        std::thread::sleep(std::time::Duration::from_secs(4));

        println!("=== player state at start: {} ===", player_state());
        let dump = |label: &str, filter: &str| match ax::ax_list_elements_filtered("Music", filter)
        {
            Ok(els) => {
                println!(
                    "--- {label} (filter={filter:?}): {} elements ---",
                    els.len()
                );
                for e in els.iter().take(60) {
                    println!("   [{}] {} enabled={:?}", e.role, e.label, e.enabled);
                }
            }
            Err(e) => println!("--- {label}: ERROR {e} ---"),
        };
        dump("after search", "Highway");
        dump("play buttons", "Play");

        // Press the first search-result row → does it navigate / play?
        println!("\n>>> pressing result 'Highway to Hell'");
        let _ = ax::ax_press_element("Music", "Highway to Hell");
        std::thread::sleep(std::time::Duration::from_secs(3));
        println!("=== player state after row press: {} ===", player_state());
        dump("detail page play", "Play");

        // Try the detail-page Play (not the toolbar one) if still stopped.
        if player_state() != "playing" {
            println!("\n>>> pressing 'Play' after navigate");
            let _ = ax::ax_press_element("Music", "Play");
            std::thread::sleep(std::time::Duration::from_secs(3));
            println!("=== player state after Play press: {} ===", player_state());
        }
    }
}
