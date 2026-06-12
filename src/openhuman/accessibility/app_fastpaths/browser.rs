//! Browser fast-path: "open `<browser>` and go to `<url>` / search / play".
//!
//! Encodes the deterministic browser flow the live transcript showed the model
//! getting wrong (tracker — Change 1.17): it re-launched the browser 6×, ran
//! `ax_interact` on the wrong app names (Chrome/Safari, never the real
//! `Brave Browser`), and finally clicked a hardcoded coordinate.
//!
//! Instead we navigate in ONE step via `open -a "<browser>" "<url>"`:
//!   - **Pure navigation** ("open Brave and go to youtube.com", "search
//!     youtube for X") completes here — no address-bar typing, no AX, no
//!     re-launching.
//!   - **Play intents** ("…and play a music video") navigate deterministically
//!     to the search-results URL, then return a non-success outcome so the
//!     general loop performs the single ambiguous "click the first result" via
//!     `vision_click` (Chromium exposes no AX tree). There's no reliable native
//!     shortcut that *selects* a search result, so we don't fake success.
//!   - **In-page media control** ("pause/next/fullscreen the video") sends the
//!     YouTube keyboard shortcut directly — the fastest possible path.
//!
//! Everything goes through the injectable [`AutomateBackend`], so the flow is
//! unit-testable with a scripted backend — no live browser, no model.

use super::browser_shortcuts::{shortcut, Browser, BrowserShortcut, Os};
use super::AutomateBackend;
use super::AutomateOutcome;

/// A resolved navigation target.
struct Destination {
    /// Fully-qualified URL to open.
    url: String,
    /// True when the goal asks to *play* something at a search URL — the
    /// fast-path navigates but then defers the final "click first result" to
    /// the general loop's `vision_click`.
    is_play: bool,
}

/// Does this (app, goal) look like a browser navigation / control request?
pub fn matches(app: &str, goal: &str) -> bool {
    resolve_browser(app, goal).is_some()
        && (extract_destination(goal).is_some()
            || extract_browser_command(goal).is_some()
            || extract_media_control(goal).is_some())
}

/// Resolve the browser's macOS display name from the `app` arg first, then the
/// goal text. Returns `None` for a generic "browser" with no named product, so
/// we never guess the wrong app (the original transcript bug).
fn resolve_browser(app: &str, goal: &str) -> Option<String> {
    // (alias substring, display name) — longest/most-specific aliases first.
    const BROWSERS: &[(&str, &str)] = &[
        ("brave", "Brave Browser"),
        ("google chrome", "Google Chrome"),
        ("chrome", "Google Chrome"),
        ("microsoft edge", "Microsoft Edge"),
        ("edge", "Microsoft Edge"),
        ("firefox", "Firefox"),
        ("safari", "Safari"),
        ("arc", "Arc"),
    ];
    let app_l = app.to_lowercase();
    for (alias, display) in BROWSERS {
        if app_l.contains(alias) {
            return Some((*display).to_string());
        }
    }
    let goal_l = goal.to_lowercase();
    for (alias, display) in BROWSERS {
        if goal_l.contains(alias) {
            return Some((*display).to_string());
        }
    }
    None
}

/// Resolve where to navigate. In priority order: YouTube intent → Google/web
/// search → a bare URL/domain mentioned in the goal. `None` if nothing matches.
fn extract_destination(goal: &str) -> Option<Destination> {
    let lower = goal.to_lowercase();
    let wants_play = has_word(&lower, "play");

    // 1. YouTube.
    if lower.contains("youtube") || lower.contains("you tube") {
        if let Some(q) = extract_query(goal) {
            return Some(Destination {
                url: format!(
                    "https://www.youtube.com/results?search_query={}",
                    percent_encode(&q)
                ),
                is_play: wants_play,
            });
        }
        // Named YouTube but no query → just open the site.
        return Some(Destination {
            url: "https://www.youtube.com".to_string(),
            is_play: false,
        });
    }

    // 2. Google / web search.
    if lower.contains("google") || lower.contains("search") {
        // "search for X" / "search X for Y" via extract_query, else the text
        // right after the word "google" ("google rust async traits").
        let q = extract_query(goal).or_else(|| {
            word_index(&lower, "google").and_then(|i| {
                let after = clean_query(&goal[i + "google".len()..]);
                (!after.is_empty()).then_some(after)
            })
        });
        if let Some(q) = q {
            return Some(Destination {
                url: format!("https://www.google.com/search?q={}", percent_encode(&q)),
                is_play: false,
            });
        }
    }

    // 3. A bare URL / domain anywhere in the goal ("go to example.com").
    if let Some(url) = extract_url(goal) {
        return Some(Destination {
            url,
            is_play: false,
        });
    }

    None
}

/// Pull a search query out of the goal: text after "for" (search phrasing) or
/// after "play", with leading articles and trailing site words stripped. `None`
/// when there's nothing searchable.
fn extract_query(goal: &str) -> Option<String> {
    let lower = goal.to_lowercase();
    // "search [youtube|the web|google] for X" / "search for X".
    if let Some(p) = lower.find(" for ") {
        let after = goal[p + " for ".len()..].trim();
        let q = clean_query(after);
        if !q.is_empty() {
            return Some(q);
        }
    }
    // "play X", "play X on youtube".
    if let Some(idx) = word_index(&lower, "play") {
        let after = goal[idx + "play".len()..].trim();
        let q = clean_query(after);
        if !q.is_empty() {
            return Some(q);
        }
    }
    None
}

/// Trim filler around an extracted query: leading articles, a leading "me",
/// and a trailing "(on|in|at) <site>" / "video(s)" tail isn't stripped (it's a
/// fine search term), but a trailing "on youtube" etc. is removed.
fn clean_query(raw: &str) -> String {
    let mut q = raw.trim().trim_end_matches(['.', '!', '?']).trim();
    // Cut at a clause boundary so "go to youtube and play X then do Y" stops at X.
    if let Some(p) = q.to_lowercase().find(" then ") {
        q = q[..p].trim();
    }
    let mut s = q.to_string();
    for lead in ["me ", "a ", "an ", "the ", "some "] {
        if let Some(rest) = strip_prefix_ci(&s, lead) {
            s = rest.trim().to_string();
            break;
        }
    }
    // Drop a trailing "(on|in) <site>" navigation tail.
    let sl = s.to_lowercase();
    for tail in [" on youtube", " in youtube", " on google", " on the web"] {
        if let Some(p) = sl.rfind(tail) {
            s.truncate(p);
            break;
        }
    }
    s.trim().to_string()
}

/// Find the first bare URL or `host.tld` token and normalize it to an
/// `https://` URL. Skips obvious non-hosts (must contain a dot, no spaces).
fn extract_url(goal: &str) -> Option<String> {
    for tok in goal.split_whitespace() {
        let t =
            tok.trim_matches(|c: char| !c.is_alphanumeric() && c != '/' && c != ':' && c != '.');
        let tl = t.to_lowercase();
        if tl.starts_with("http://") || tl.starts_with("https://") {
            return Some(t.to_string());
        }
        // host.tld with a plausible TLD and no scheme.
        if t.contains('.') && !t.contains('/') && looks_like_domain(&tl) {
            return Some(format!("https://{t}"));
        }
    }
    None
}

/// Crude domain check: has a dot, and the last label is 2–24 ascii letters.
fn looks_like_domain(s: &str) -> bool {
    match s.rsplit_once('.') {
        Some((host, tld)) => {
            !host.is_empty()
                && (2..=24).contains(&tld.len())
                && tld.chars().all(|c| c.is_ascii_alphabetic())
        }
        None => false,
    }
}

/// Map a browser-level command ("new tab", "go back", "reload", "switch to tab
/// 3", …) to a [`BrowserShortcut`]. Resolved to actual keys per browser+OS by
/// [`browser_shortcuts::shortcut`]. Navigation (a URL/search) is handled earlier
/// by [`extract_destination`]; this covers tab/window/history/zoom verbs.
fn extract_browser_command(goal: &str) -> Option<BrowserShortcut> {
    use BrowserShortcut as S;
    let l = goal.to_lowercase();

    // "switch to tab 3" / "go to tab 5" — a digit must follow "tab".
    if let Some(n) = tab_number(&l) {
        return Some(S::TabN(n));
    }
    // Most-specific phrases first.
    if l.contains("hard reload") || l.contains("force reload") || l.contains("hard refresh") {
        Some(S::HardReload)
    } else if l.contains("reload") || l.contains("refresh") {
        Some(S::Reload)
    } else if l.contains("new tab") {
        Some(S::NewTab)
    } else if l.contains("close tab")
        || l.contains("close this tab")
        || l.contains("close the tab")
        || l.contains("close current tab")
    {
        Some(S::CloseTab)
    } else if l.contains("reopen") || l.contains("restore tab") || l.contains("undo close") {
        Some(S::ReopenTab)
    } else if l.contains("incognito")
        || l.contains("inprivate")
        || l.contains("private window")
        || l.contains("private browsing")
    {
        Some(S::PrivateWindow)
    } else if l.contains("new window") {
        Some(S::NewWindow)
    } else if l.contains("next tab") {
        Some(S::NextTab)
    } else if l.contains("previous tab") || l.contains("prev tab") {
        Some(S::PrevTab)
    } else if l.contains("last tab") {
        Some(S::LastTab)
    } else if l.contains("go back") || l.contains("navigate back") {
        Some(S::Back)
    } else if l.contains("go forward") || l.contains("navigate forward") {
        Some(S::Forward)
    } else if l.contains("address bar") || l.contains("url bar") || l.contains("location bar") {
        Some(S::FocusAddressBar)
    } else if has_word(&l, "history") {
        Some(S::History)
    } else if has_word(&l, "downloads") || l.contains("download list") {
        Some(S::Downloads)
    } else if l.contains("bookmark this")
        || l.contains("bookmark the page")
        || l.contains("bookmark page")
        || l.contains("add bookmark")
        || l.contains("save bookmark")
    {
        Some(S::BookmarkPage)
    } else if l.contains("zoom in") {
        Some(S::ZoomIn)
    } else if l.contains("zoom out") {
        Some(S::ZoomOut)
    } else if l.contains("reset zoom") || l.contains("actual size") || l.contains("default zoom") {
        Some(S::ZoomReset)
    } else {
        None
    }
}

/// Extract a 1‑9 tab number following the word "tab" ("switch to tab 3"). Also
/// accepts "tab number 3". `None` when no digit follows (so "new tab" / "next
/// tab" don't match).
fn tab_number(lower: &str) -> Option<u8> {
    let idx = lower.find("tab ")?;
    let rest = lower[idx + "tab ".len()..].trim_start();
    let mut toks = rest.split_whitespace();
    let first = toks.next()?;
    let digit = if first == "number" || first == "no" || first == "#" {
        toks.next()?
    } else {
        first
    };
    let n: u8 = digit
        .trim_matches(|c: char| !c.is_ascii_digit())
        .parse()
        .ok()?;
    (1..=9).contains(&n).then_some(n)
}

/// Map an in-page media-control goal to a YouTube keyboard shortcut, if any.
fn extract_media_control(goal: &str) -> Option<Vec<String>> {
    let l = goal.to_lowercase();
    // Only treat as a *control* command when there's no navigation verb.
    if l.contains("open ") || l.contains("go to") || l.contains("search") {
        return None;
    }
    let key = |k: &str| Some(vec![k.to_string()]);
    if has_word(&l, "fullscreen") || l.contains("full screen") {
        key("f")
    } else if has_word(&l, "mute") || has_word(&l, "unmute") {
        key("m")
    } else if has_word(&l, "next") || has_word(&l, "skip") {
        Some(vec!["shift".to_string(), "n".to_string()])
    } else if has_word(&l, "previous") || has_word(&l, "back") {
        Some(vec!["shift".to_string(), "p".to_string()])
    } else if has_word(&l, "pause") || has_word(&l, "resume") || has_word(&l, "play") {
        // YouTube `k` toggles play/pause.
        key("k")
    } else {
        None
    }
}

/// Run the browser fast-path.
pub async fn run(app: &str, goal: &str, backend: &dyn AutomateBackend) -> AutomateOutcome {
    use super::super::automate::progress;
    use crate::openhuman::overlay::OverlayAttentionTone;

    let mut steps: Vec<String> = Vec::new();
    let browser = match resolve_browser(app, goal) {
        Some(b) => b,
        None => return fail("no recognizable browser", steps),
    };

    // Navigation takes priority over in-page control.
    if let Some(dest) = extract_destination(goal) {
        log::info!(
            "[automate::browser] ▶ open browser={browser:?} url={:?} is_play={}",
            dest.url,
            dest.is_play
        );
        progress(format!("Opening {browser}…"), OverlayAttentionTone::Accent);

        match backend.open_url_in_app(&browser, &dest.url).await {
            Ok(m) => steps.push(format!("navigate: {m}")),
            Err(e) => {
                steps.push(format!("navigate FAILED: {e}"));
                return fail("could not open the browser/URL", steps);
            }
        }
        backend.settle(&browser).await;
        // Give the page network time to render before any follow-up.
        backend.wait(1200).await;

        if dest.is_play {
            // Deterministic part done; defer the single "click first result" to
            // the general loop's vision_click (no reliable shortcut selects a
            // search result). Returning non-success makes the loop take over —
            // it does NOT re-launch from scratch, since the page is already up.
            steps.push("navigated to search results; deferring play-click to vision".to_string());
            return fail(
                "navigated; first-result click deferred to general loop",
                steps,
            );
        }

        return AutomateOutcome {
            success: true,
            summary: format!("Opened {} in {browser}.", dest.url),
            steps,
        };
    }

    // Browser-level command ("new tab", "go back", "reload", "tab 3") via the
    // cross-platform shortcut table — no AX, no navigation.
    if let Some(intent) = extract_browser_command(goal) {
        let keys = shortcut(intent, Browser::from_display(&browser), Os::current());
        let combo = keys.join("+");
        log::info!("[automate::browser] ▶ command {intent:?} browser={browser:?} keys={combo}");
        progress(format!("Pressing {combo}…"), OverlayAttentionTone::Accent);
        match backend.key(&keys).await {
            Ok(m) => {
                steps.push(format!("hotkey {combo}: {m}"));
                return AutomateOutcome {
                    success: true,
                    summary: format!("Sent {combo} to {browser}."),
                    steps,
                };
            }
            Err(e) => {
                steps.push(format!("hotkey FAILED: {e}"));
                return fail("could not send the browser shortcut", steps);
            }
        }
    }

    // In-page media control via a YouTube keyboard shortcut.
    if let Some(keys) = extract_media_control(goal) {
        let combo = keys.join("+");
        log::info!("[automate::browser] ▶ media control browser={browser:?} keys={combo}");
        progress(format!("Pressing {combo}…"), OverlayAttentionTone::Accent);
        match backend.key(&keys).await {
            Ok(m) => {
                steps.push(format!("hotkey {combo}: {m}"));
                return AutomateOutcome {
                    success: true,
                    summary: format!("Sent {combo} to {browser}."),
                    steps,
                };
            }
            Err(e) => {
                steps.push(format!("hotkey FAILED: {e}"));
                return fail("could not send the media shortcut", steps);
            }
        }
    }

    fail("no browser destination or control in goal", steps)
}

fn fail(msg: &str, steps: Vec<String>) -> AutomateOutcome {
    AutomateOutcome {
        success: false,
        summary: format!("Browser fast-path: {msg}"),
        steps,
    }
}

// ── small string helpers ────────────────────────────────────────────────────

/// True if `needle` appears as a whole word in `haystack` (already lowercased).
fn has_word(haystack: &str, needle: &str) -> bool {
    word_index(haystack, needle).is_some()
}

/// Byte index of `needle` as a whole word in `haystack` (already lowercased).
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

/// Case-insensitive `strip_prefix`.
fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    if s.len() >= prefix.len()
        && s.is_char_boundary(prefix.len())
        && s[..prefix.len()].to_lowercase() == prefix.to_lowercase()
    {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}

/// Percent-encode reserved characters in a query value (enough for a `?q=`
/// search param; not a full RFC-3986 encoder). Mirrors `music::percent_encode`.
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

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn resolve_browser_from_app_then_goal_never_guesses() {
        // From the app arg.
        assert_eq!(
            resolve_browser("Brave Browser", "do something").as_deref(),
            Some("Brave Browser")
        );
        // From the goal text.
        assert_eq!(
            resolve_browser("", "open chrome and go to x.com").as_deref(),
            Some("Google Chrome")
        );
        assert_eq!(
            resolve_browser("", "use safari to open apple.com").as_deref(),
            Some("Safari")
        );
        assert_eq!(
            resolve_browser("", "open microsoft edge").as_deref(),
            Some("Microsoft Edge")
        );
        // Generic "browser" with no named product → None (the original bug: we
        // must NOT guess Chrome/Safari).
        assert_eq!(resolve_browser("", "open my browser"), None);
    }

    #[test]
    fn matches_requires_browser_and_destination_or_control() {
        assert!(matches("Brave Browser", "go to youtube.com"));
        assert!(matches("", "open chrome and search youtube for lofi"));
        assert!(matches("Brave Browser", "pause the video"));
        // Browser but no destination/control.
        assert!(!matches("Brave Browser", "hello there"));
        // Destination but no resolvable browser.
        assert!(!matches("Slack", "go to youtube.com"));
    }

    #[test]
    fn destination_youtube_search_encodes_query() {
        let d = extract_destination("go to youtube and play a music video").unwrap();
        assert_eq!(
            d.url,
            "https://www.youtube.com/results?search_query=music%20video"
        );
        assert!(d.is_play);
    }

    #[test]
    fn destination_youtube_search_for_phrasing() {
        let d = extract_destination("search youtube for lofi beats").unwrap();
        assert_eq!(
            d.url,
            "https://www.youtube.com/results?search_query=lofi%20beats"
        );
        assert!(!d.is_play); // no "play" word → navigation only
    }

    #[test]
    fn destination_bare_youtube_no_query() {
        let d = extract_destination("open youtube").unwrap();
        assert_eq!(d.url, "https://www.youtube.com");
        assert!(!d.is_play);
    }

    #[test]
    fn destination_bare_domain_normalized_to_https() {
        let d = extract_destination("go to example.com").unwrap();
        assert_eq!(d.url, "https://example.com");
        let d2 = extract_destination("open https://news.ycombinator.com").unwrap();
        assert_eq!(d2.url, "https://news.ycombinator.com");
    }

    #[test]
    fn destination_google_search() {
        let d = extract_destination("google rust async traits").unwrap();
        assert_eq!(
            d.url,
            "https://www.google.com/search?q=rust%20async%20traits"
        );
    }

    #[test]
    fn destination_none_when_no_target() {
        assert!(extract_destination("just hang out").is_none());
    }

    #[test]
    fn media_control_maps_to_youtube_shortcuts() {
        assert_eq!(
            extract_media_control("pause the video"),
            Some(vec!["k".into()])
        );
        assert_eq!(
            extract_media_control("resume playback"),
            Some(vec!["k".into()])
        );
        assert_eq!(extract_media_control("mute it"), Some(vec!["m".into()]));
        assert_eq!(
            extract_media_control("next video"),
            Some(vec!["shift".into(), "n".into()])
        );
        assert_eq!(
            extract_media_control("go fullscreen"),
            Some(vec!["f".into()])
        );
        // A navigation goal is NOT a control command.
        assert_eq!(extract_media_control("open youtube and play lofi"), None);
    }

    #[test]
    fn word_index_is_whole_word() {
        // "play" must not match inside "display"/"playback".
        assert!(has_word("play a song", "play"));
        assert!(!has_word("display settings", "play"));
        assert!(!has_word("open playback options", "play"));
    }

    #[test]
    fn browser_command_maps_common_verbs() {
        use BrowserShortcut as S;
        assert_eq!(extract_browser_command("open a new tab"), Some(S::NewTab));
        assert_eq!(extract_browser_command("close this tab"), Some(S::CloseTab));
        assert_eq!(
            extract_browser_command("reopen the closed tab"),
            Some(S::ReopenTab)
        );
        assert_eq!(extract_browser_command("go back"), Some(S::Back));
        assert_eq!(extract_browser_command("reload the page"), Some(S::Reload));
        assert_eq!(extract_browser_command("hard reload"), Some(S::HardReload));
        assert_eq!(extract_browser_command("next tab"), Some(S::NextTab));
        assert_eq!(
            extract_browser_command("open an incognito window"),
            Some(S::PrivateWindow)
        );
        assert_eq!(extract_browser_command("show my history"), Some(S::History));
        assert_eq!(extract_browser_command("zoom in"), Some(S::ZoomIn));
        // No browser command.
        assert_eq!(extract_browser_command("play a music video"), None);
    }

    #[test]
    fn tab_number_only_with_digit() {
        assert_eq!(tab_number("switch to tab 3"), Some(3));
        assert_eq!(tab_number("go to tab number 5"), Some(5));
        // "new tab"/"next tab" have no trailing digit → not a tab-number jump.
        assert_eq!(tab_number("open a new tab"), None);
        assert_eq!(tab_number("next tab"), None);
        assert_eq!(
            extract_browser_command("switch to tab 4"),
            Some(BrowserShortcut::TabN(4))
        );
    }
}

/// Live integration test — drives a real browser. Ignored by default (needs a
/// browser installed + Accessibility/Screen-recording permission). Asserts
/// tool-level success only; the visual page state is best-effort.
///
///   cargo test --lib browser_fastpath_live -- --ignored --nocapture
#[cfg(all(test, target_os = "macos"))]
mod live {
    use super::run;
    use crate::openhuman::accessibility::automate::RealBackend;

    #[tokio::test]
    #[ignore = "requires macOS + a browser + Accessibility permission"]
    async fn browser_fastpath_live() {
        let backend = RealBackend::new(crate::openhuman::config::Config::default());
        // Use Safari (always present on macOS) for a deterministic nav.
        let out = run("Safari", "open safari and go to example.com", &backend).await;
        println!(
            "[browser_fastpath_live] success={} summary={}",
            out.success, out.summary
        );
        for s in &out.steps {
            println!("  - {s}");
        }
        assert!(
            out.success,
            "nav fast-path reported failure: {}",
            out.summary
        );
    }
}
