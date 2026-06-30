//! `automate` — Rust-driven multi-step UI automation loop.
//!
//! Phase 1.5 (see `docs/voice-automate-plan.md`). The chat orchestrator calls
//! `automate{app, goal}` **once**; this module then runs the whole multi-step
//! flow internally with a *fast* model, so the heavy chat model never sits
//! inside the click loop. Each iteration is **perceive → decide → act →
//! settle → verify**:
//!
//!   - **perceive** — read a small, filtered accessibility snapshot of the app
//!     (`ax_interact::ax_list_elements_filtered`, capped — never a raw dump,
//!     which is what made the chat model hallucinate; tracker §1.13).
//!   - **decide** — ask the fast model for exactly one JSON action.
//!   - **act**     — run it via the existing AX primitives / `launch_app`.
//!   - **settle**  — wait for the UI to stop changing (M2 makes this real; the
//!     M1 backend uses a short fixed wait).
//!   - **verify**  — fold the post-action snapshot back into the next prompt.
//!
//! The loop is generic over an [`AutomateBackend`] so the decision model, the
//! accessibility calls, and the launcher are all injectable — the unit tests
//! drive a scripted backend with no mic, no AX tree, and no LLM.

use super::ax_interact as ax;
use crate::openhuman::overlay::{publish_attention, OverlayAttentionEvent, OverlayAttentionTone};
use async_trait::async_trait;
use serde::Deserialize;

const LOG_PREFIX: &str = "[automate]";

/// Push a one-line progress message to the notch / overlay so the user sees the
/// automation happening live (M4). Fire-and-forget: a no-op when nothing is
/// subscribed (e.g. unit tests, or the notch window isn't running).
pub(crate) fn progress(message: impl Into<String>, tone: OverlayAttentionTone) {
    let _ = publish_attention(
        OverlayAttentionEvent::new(message)
            .with_source("automate")
            .with_tone(tone)
            .with_ttl_ms(5000),
    );
}

/// Default ceiling on loop iterations. Each iteration is one fast-model call
/// plus one action, so this bounds latency and cost even if the model never
/// emits `done`.
pub const DEFAULT_STEP_BUDGET: u32 = 12;

/// How many elements a perceive snapshot renders into the prompt. Mirrors the
/// `ax_interact` tool cap so a broad/empty filter can't overflow the model's
/// context and trigger the truncation→hallucination failure (tracker §1.13).
const MAX_SNAPSHOT: usize = 40;

/// One decoded action from the fast model.
#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
pub struct Action {
    /// The model's short reasoning. Logged, never executed.
    #[serde(default)]
    pub thought: String,
    /// One of: `launch`, `list`, `press`, `set_value`, `done`, `fail`.
    pub action: String,
    /// Optional per-action app override; defaults to the task's app.
    #[serde(default)]
    pub app: Option<String>,
    /// Substring filter for `list`.
    #[serde(default)]
    pub filter: String,
    /// Element label for `press` / `set_value`.
    #[serde(default)]
    pub label: String,
    /// Text to enter for `set_value`.
    #[serde(default)]
    pub value: String,
    /// Natural-language target for `vision_click` (e.g. "the green Call button").
    #[serde(default)]
    pub description: String,
    /// Key chord / single key for `hotkey` (e.g. `["Cmd","L"]`, `["/"]`).
    #[serde(default)]
    pub keys: Vec<String>,
    /// Final message for `done` / `fail`.
    #[serde(default)]
    pub summary: String,
}

/// The result of a completed (or budget-exhausted) automation run.
#[derive(Debug, Clone, PartialEq)]
pub struct AutomateOutcome {
    pub success: bool,
    pub summary: String,
    /// One human-readable line per executed step — surfaced back to the chat
    /// agent and useful in logs.
    pub steps: Vec<String>,
}

impl AutomateOutcome {
    fn fail(summary: impl Into<String>, steps: Vec<String>) -> Self {
        Self {
            success: false,
            summary: summary.into(),
            steps,
        }
    }
}

/// Injectable side-effects for the loop. The production impl
/// ([`RealBackend`]) talks to the OS accessibility tree and a fast LLM; tests
/// supply a scripted impl.
#[async_trait]
pub trait AutomateBackend: Send + Sync {
    /// Read interactive elements in `app` whose label contains `filter`.
    async fn perceive(&self, app: &str, filter: &str) -> Result<Vec<ax::AXElement>, String>;
    /// Ask the decision model for one JSON action. `system` pins the schema;
    /// `user` carries the goal + current snapshot + recent step history.
    async fn decide(&self, system: &str, user: &str) -> Result<String, String>;
    async fn act_launch(&self, app: &str) -> Result<String, String>;
    async fn act_press(&self, app: &str, label: &str) -> Result<String, String>;
    async fn act_set_value(&self, app: &str, label: &str, value: &str) -> Result<String, String>;
    /// Open a URL / URI-scheme (e.g. `music://…search?term=…`) via the OS opener.
    /// Used by deterministic app fast-paths; the general loop does not call it.
    async fn open_url(&self, url: &str) -> Result<String, String>;
    /// Open a URL in a **specific** app (e.g. a chosen browser) so navigation
    /// lands in the app the user named — `open_url` uses the *default* handler,
    /// which would send a `https://` link to whatever the default browser is.
    /// Default delegates to [`open_url`](Self::open_url) so non-browser backends
    /// stay correct. Used by the browser fast-path.
    async fn open_url_in_app(&self, _app: &str, url: &str) -> Result<String, String> {
        self.open_url(url).await
    }
    /// Send a keyboard chord (`["Cmd","L"]`) or a single key (`["/"]`) to the
    /// frontmost app. Lets fast-paths and the loop drive app shortcuts (focus
    /// the address bar, YouTube `/` search, `k`/space play-pause) instead of
    /// hunting AX labels. Default errors so input-less backends can't actuate.
    async fn key(&self, _keys: &[String]) -> Result<String, String> {
        Err("keyboard unsupported by this backend".to_string())
    }
    /// Type literal text into the frontmost app. Default errors (see [`key`]).
    async fn type_text(&self, _text: &str) -> Result<String, String> {
        Err("typing unsupported by this backend".to_string())
    }
    /// Best-effort: is media currently playing? `None` when the backend can't
    /// tell (non-macOS, or not applicable). Media fast-paths use this to confirm
    /// an action *actually started playback* rather than just succeeding at the
    /// AX level — the false-success that made "play" silently no-op (§1.11).
    async fn verify_playing(&self) -> Option<bool> {
        None
    }
    /// The currently-playing track as `(name, artist)`, if the backend can read
    /// it. Used by the Music fast-path to confirm it played the *right* track
    /// (the AX row label carries only the title, so "Numb" can resolve to the
    /// wrong artist — see tracker §1.x). `None` when unknown (non-macOS, nothing
    /// playing, or not applicable).
    async fn now_playing(&self) -> Option<(String, String)> {
        None
    }
    /// Capture the target app's window + the geometry needed to map a click
    /// from image pixels to screen points. Used by the `vision_click` fallback
    /// for apps with no usable accessibility tree (Electron/Chromium). Default
    /// errors so backends without screen access (tests, headless) opt out.
    async fn screenshot(
        &self,
        _app: &str,
    ) -> Result<(String, super::vision_click::CaptureGeometry), String> {
        Err("screenshot unsupported by this backend".to_string())
    }
    /// Ask the vision model for the absolute *screen* coordinates of the
    /// described element in `screenshot`. `Ok(None)` = not visible. Default
    /// `Ok(None)` so non-vision backends never click.
    async fn locate(
        &self,
        _screenshot: &str,
        _geom: &super::vision_click::CaptureGeometry,
        _description: &str,
    ) -> Result<Option<(i32, i32)>, String> {
        Ok(None)
    }
    /// Name of the frontmost application, if known. Used as the §1.8 safety
    /// guard: a `vision_click` only fires when the target app is frontmost, so
    /// synthetic input never lands on OpenHuman's own window. `None` = unknown.
    async fn frontmost_app(&self) -> Option<String> {
        None
    }
    /// Issue a single guarded left-click at absolute screen coordinates. Default
    /// errors so backends without input access can't click.
    async fn click(&self, _x: i32, _y: i32) -> Result<String, String> {
        Err("click unsupported by this backend".to_string())
    }
    /// Block until the UI settles after an action.
    async fn settle(&self, app: &str);
    /// Wait ~`ms` of real time. Used by fast-paths to let asynchronous content
    /// (e.g. network search results) render between perceive attempts. Default
    /// is a real sleep; test backends override it to a no-op so suites stay fast.
    async fn wait(&self, ms: u64) {
        tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
    }
}

/// Tuning for a run.
#[derive(Debug, Clone, Copy)]
pub struct AutomateOptions {
    pub step_budget: u32,
}

impl Default for AutomateOptions {
    fn default() -> Self {
        Self {
            step_budget: DEFAULT_STEP_BUDGET,
        }
    }
}

/// System prompt pinning the action contract for the fast model.
fn system_prompt() -> String {
    "You drive a desktop app's UI to accomplish a goal. You see a list of the \
     app's interactive elements (each as `[role] label`) and act one step at a \
     time.\n\
     \n\
     Respond with EXACTLY ONE JSON object and nothing else:\n\
     {\"thought\":\"...\",\"action\":\"<verb>\",\"app\":\"<optional>\",\
     \"filter\":\"...\",\"label\":\"...\",\"value\":\"...\",\"keys\":[],\"summary\":\"...\"}\n\
     \n\
     Verbs:\n\
     • launch     — open the app (use first if it isn't showing any elements)\n\
     • list       — re-read elements; set `filter` to a substring to narrow them\n\
     • press      — activate the element whose label matches `label`\n\
     • set_value  — type `value` into the field matching `label` (omit label = first field)\n\
     • hotkey     — send an app keyboard shortcut; put the chord in `keys` (modifiers \
     first, e.g. [\"Cmd\",\"L\"] to focus a browser address bar, [\"Cmd\",\"T\"] new tab) \
     or a single key (e.g. [\"/\"] to focus YouTube search, [\"k\"] play/pause, [\"f\"] \
     fullscreen). Prefer a known shortcut over hunting labels or clicking.\n\
     • vision_click — click an element by sight; put a short `description` of the \
     target (e.g. 'the green Call button'). Use this when the element list is \
     EMPTY or missing your target — common for Electron/Chromium apps (browsers, \
     Slack, Discord, VS Code) that expose no accessibility tree.\n\
     • done       — goal achieved; put a short result in `summary`\n\
     • fail       — goal cannot be achieved; explain in `summary`\n\
     \n\
     Rules:\n\
     - Pressing a LIST ROW or SEARCH RESULT usually only selects/opens it. To \
     trigger playback or submission you must then press the actual action button \
     (e.g. open a song, THEN press its 'Play'). After such a press, `list` again \
     to see the new screen.\n\
     - Prefer an exact label match. Keep `filter` specific so the snapshot stays small.\n\
     - For browsers and web apps, prefer `hotkey` for navigation and media control \
     (address bar, search focus, play/pause/next) — it's faster and more reliable \
     than clicking, and works even when the accessibility tree is empty.\n\
     - If the app shows NO elements, prefer `hotkey` (if a known shortcut applies) \
     or `vision_click` with a clear `description` over guessing labels.\n\
     - Output JSON only — no prose, no code fences."
        .to_string()
}

/// Render a perceive snapshot into compact prompt text.
fn render_snapshot(app: &str, filter: &str, elements: &[ax::AXElement]) -> String {
    if elements.is_empty() {
        return format!(
            "App '{app}' shows no elements matching filter '{filter}' (it may still be \
             loading, or needs launching)."
        );
    }
    let shown = elements.len().min(MAX_SNAPSHOT);
    let mut out = format!(
        "App '{app}' elements (filter '{filter}', showing {shown} of {}):\n",
        elements.len()
    );
    for e in elements.iter().take(MAX_SNAPSHOT) {
        // NB: we don't annotate `enabled` here — AXEnabled is unreliable
        // per-app (Apple Music marks pressable rows disabled), so surfacing it
        // would mislead the model into avoiding real controls.
        out.push_str(&format!("  [{}] {}\n", e.role, e.label));
    }
    out
}

/// Parse one action from raw model text, tolerating code fences and surrounding
/// prose by extracting the first balanced `{...}` block. Returns `Err` so the
/// caller can issue a single repair retry before giving up — we never *act* on
/// an unparseable guess (tracker §1.13 hallucination lesson).
fn parse_action(raw: &str) -> Result<Action, String> {
    let trimmed = raw.trim();
    if let Ok(a) = serde_json::from_str::<Action>(trimmed) {
        return Ok(a);
    }
    // Extract the first {...} span and retry.
    if let (Some(start), Some(end)) = (trimmed.find('{'), trimmed.rfind('}')) {
        if end > start {
            if let Ok(a) = serde_json::from_str::<Action>(&trimmed[start..=end]) {
                return Ok(a);
            }
        }
    }
    Err(format!(
        "could not parse an action from model output: {trimmed:?}"
    ))
}

/// Run the automation loop until the goal is met, it fails, or the step budget
/// is exhausted.
pub async fn run(
    app: &str,
    goal: &str,
    backend: &dyn AutomateBackend,
    opts: AutomateOptions,
) -> AutomateOutcome {
    log::info!(
        "{LOG_PREFIX} ▶ run app={app:?} goal={goal:?} budget={}",
        opts.step_budget
    );

    // Foreground the target app FIRST, always. This guarantees the app is
    // frontmost before we perceive or act — so AX reads the right window and any
    // synthetic input (keyboard/mouse) lands on it, not on OpenHuman's own
    // window (which is what crashed CEF in §1.8). `act_launch` is `open -a`,
    // which both opens and activates; idempotent if already running.
    match backend.act_launch(app).await {
        Ok(m) => log::info!("{LOG_PREFIX} foregrounded: {m}"),
        Err(e) => log::warn!("{LOG_PREFIX} foreground failed for {app:?}: {e}"),
    }
    backend.settle(app).await;

    // Deterministic accelerator: if a known app + intent has a proven native
    // sequence, run it first. On `None` (no fast-path) or a failed fast-path we
    // fall through to the general model-driven loop — so the fast-path can only
    // help, never block. (Structurally different from the removed `play_music`
    // tool, §1.13: this is internal to `automate`, not a tool the LLM selects.)
    if let Some(outcome) = super::app_fastpaths::try_fastpath(app, goal, backend).await {
        if outcome.success {
            log::info!("{LOG_PREFIX} fast-path succeeded for app={app:?}");
            return outcome;
        }
        log::info!("{LOG_PREFIX} fast-path did not complete; falling through to general loop");
    }

    let system = system_prompt();
    let mut steps: Vec<String> = Vec::new();
    let mut last_filter = String::new();
    // One repair retry budget for unparseable model output.
    let mut repair_left = 1u32;
    // No-progress guard: track the last actionable signature so a model that
    // keeps issuing the same call (e.g. pressing 'Search' over and over) bails
    // instead of burning the whole step budget.
    let mut last_sig = String::new();
    let mut repeat_count = 0u32;
    // Most recent rendered snapshot — surfaced in terminal failure responses so
    // the agent sees what was actually on screen (instead of a bare "budget
    // exhausted"), and can pick a real label next time.
    let mut last_snapshot = String::new();

    for step in 0..opts.step_budget {
        // ── perceive ──
        let snapshot = match backend.perceive(app, &last_filter).await {
            Ok(els) => render_snapshot(app, &last_filter, &els),
            Err(e) => {
                log::warn!("{LOG_PREFIX} perceive failed: {e}");
                format!("(perceive error: {e})")
            }
        };
        last_snapshot = snapshot.clone();

        // ── decide ──
        let user = format!(
            "Goal: {goal}\nApp: {app}\n\nCurrent screen:\n{snapshot}\n\nSteps so far:\n{}\n\n\
             Reply with the next single JSON action.",
            if steps.is_empty() {
                "  (none yet)".to_string()
            } else {
                steps
                    .iter()
                    .map(|s| format!("  - {s}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        );
        let raw = match backend.decide(&system, &user).await {
            Ok(t) => t,
            Err(e) => {
                log::warn!("{LOG_PREFIX} decide failed: {e}");
                return AutomateOutcome::fail(format!("decision model error: {e}"), steps);
            }
        };

        let action = match parse_action(&raw) {
            Ok(a) => a,
            Err(e) => {
                if repair_left > 0 {
                    repair_left -= 1;
                    log::warn!("{LOG_PREFIX} step={step} unparseable action, retrying: {e}");
                    steps.push("(model produced unparseable output; retried)".to_string());
                    continue;
                }
                return AutomateOutcome::fail(format!("model output unparseable: {e}"), steps);
            }
        };

        let target_app = action
            .app
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(app);
        log::info!(
            "{LOG_PREFIX} step={step} action={:?} app={target_app:?} label={:?} filter={:?}",
            action.action,
            action.label,
            action.filter
        );

        // ── no-progress guard ──
        if !matches!(action.action.as_str(), "done" | "fail") {
            let sig = format!(
                "{}|{}|{}|{}|{}",
                action.action,
                action.label,
                action.filter,
                action.description,
                action.keys.join("+")
            );
            if sig == last_sig {
                repeat_count += 1;
            } else {
                repeat_count = 0;
                last_sig = sig;
            }
            // initial + 2 repeats = 3 identical actions in a row.
            if repeat_count >= 2 {
                log::warn!("{LOG_PREFIX} no progress: action repeated 3× ({last_sig}); aborting");
                steps.push(format!(
                    "aborted: repeated '{}' 3× with no progress",
                    action.action
                ));
                return AutomateOutcome::fail(
                    format!(
                        "Stuck repeating '{}' with no progress — that action isn't advancing the goal.{} Switch tactics: pick a specific label from the screen, take a screenshot + vision_click, or use a keyboard shortcut.",
                        action.action,
                        screen_hint(&last_snapshot),
                    ),
                    steps,
                );
            }
        }

        // ── act ──
        match action.action.as_str() {
            "done" => {
                let summary = if action.summary.is_empty() {
                    "Goal completed.".to_string()
                } else {
                    action.summary.clone()
                };
                log::info!("{LOG_PREFIX} ✓ done: {summary}");
                progress(&summary, OverlayAttentionTone::Success);
                return AutomateOutcome {
                    success: true,
                    summary,
                    steps,
                };
            }
            "fail" => {
                let summary = if action.summary.is_empty() {
                    "Goal could not be completed.".to_string()
                } else {
                    action.summary.clone()
                };
                log::info!("{LOG_PREFIX} ✗ model gave up: {summary}");
                progress(&summary, OverlayAttentionTone::Neutral);
                return AutomateOutcome::fail(summary, steps);
            }
            "list" => {
                last_filter = action.filter.clone();
                steps.push(format!("list filter={:?}", last_filter));
            }
            "launch" => {
                progress(
                    format!("Opening {target_app}…"),
                    OverlayAttentionTone::Accent,
                );
                match backend.act_launch(target_app).await {
                    Ok(msg) => steps.push(format!("launch: {msg}")),
                    Err(e) => steps.push(format!("launch FAILED: {e}")),
                }
                backend.settle(target_app).await;
            }
            "press" => {
                if action.label.trim().is_empty() {
                    steps.push("press skipped: empty label".to_string());
                    continue;
                }
                progress(
                    format!("Pressing {}…", action.label),
                    OverlayAttentionTone::Accent,
                );
                match backend.act_press(target_app, &action.label).await {
                    Ok(msg) => steps.push(format!("press: {msg}")),
                    Err(e) => steps.push(format!("press FAILED: {e}")),
                }
                backend.settle(target_app).await;
            }
            "set_value" => {
                if action.value.is_empty() {
                    steps.push("set_value skipped: empty value".to_string());
                    continue;
                }
                progress("Typing…", OverlayAttentionTone::Accent);
                match backend
                    .act_set_value(target_app, &action.label, &action.value)
                    .await
                {
                    Ok(msg) => steps.push(format!("set_value: {msg}")),
                    Err(e) => steps.push(format!("set_value FAILED: {e}")),
                }
                backend.settle(target_app).await;
            }
            "hotkey" => {
                if action.keys.is_empty() {
                    steps.push("hotkey skipped: no keys".to_string());
                    continue;
                }
                let combo = action.keys.join("+");
                progress(format!("Pressing {combo}…"), OverlayAttentionTone::Accent);
                match backend.key(&action.keys).await {
                    Ok(msg) => steps.push(format!("hotkey: {msg}")),
                    Err(e) => steps.push(format!("hotkey FAILED: {e}")),
                }
                backend.settle(target_app).await;
            }
            "vision_click" => {
                let description = action.description.trim();
                if description.is_empty() {
                    steps.push("vision_click skipped: empty description".to_string());
                    continue;
                }
                // ── §1.8 safety guard ──
                // Only click when the target app is frontmost, so synthetic
                // input never lands on OpenHuman's own window (the CEF crash).
                // `None` = can't tell → proceed best-effort (the loop already
                // foregrounded the app at start). We only REFUSE on positive
                // evidence that a different app is focused.
                if let Some(front) = backend.frontmost_app().await {
                    if !front.eq_ignore_ascii_case(target_app) {
                        log::warn!(
                            "{LOG_PREFIX} vision_click: {target_app:?} not frontmost ({front:?}); re-foregrounding"
                        );
                        let _ = backend.act_launch(target_app).await;
                        backend.settle(target_app).await;
                        let still_wrong = backend
                            .frontmost_app()
                            .await
                            .map(|f| !f.eq_ignore_ascii_case(target_app))
                            .unwrap_or(false);
                        if still_wrong {
                            steps.push(format!(
                                "vision_click refused: {target_app} is not frontmost"
                            ));
                            continue;
                        }
                    }
                }
                progress(
                    format!("Looking for {description}…"),
                    OverlayAttentionTone::Accent,
                );
                let (shot, geom) = match backend.screenshot(target_app).await {
                    Ok(pair) => pair,
                    Err(e) => {
                        steps.push(format!("vision_click FAILED: screenshot: {e}"));
                        continue;
                    }
                };
                match backend.locate(&shot, &geom, description).await {
                    Ok(Some((x, y))) => {
                        progress(
                            format!("Clicking {description}…"),
                            OverlayAttentionTone::Accent,
                        );
                        match backend.click(x, y).await {
                            Ok(msg) => steps.push(format!("vision_click: {msg}")),
                            Err(e) => steps.push(format!("vision_click FAILED: click: {e}")),
                        }
                        backend.settle(target_app).await;
                    }
                    Ok(None) => {
                        steps.push(format!("vision_click: '{description}' not found on screen"));
                    }
                    Err(e) => {
                        steps.push(format!("vision_click FAILED: locate: {e}"));
                    }
                }
            }
            other => {
                steps.push(format!("unknown action {other:?} ignored"));
            }
        }
    }

    log::info!("{LOG_PREFIX} step budget ({}) exhausted", opts.step_budget);
    AutomateOutcome::fail(
        format!(
            "Step budget ({}) exhausted before the goal was confirmed complete.{} Try a different approach (a screenshot/vision_click, a known keyboard shortcut, or a more specific filter) — repeating the same steps won't help.",
            opts.step_budget,
            screen_hint(&last_snapshot),
        ),
        steps,
    )
}

/// A compact " On screen: [role] a, [role] b, …" hint built from the last
/// rendered snapshot, for failure responses. Empty when there's nothing useful.
fn screen_hint(snapshot: &str) -> String {
    let labels: Vec<&str> = snapshot
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with('[') || l.starts_with("• ["))
        .take(10)
        .collect();
    if labels.is_empty() {
        String::new()
    } else {
        format!(" On screen: {}.", labels.join("; "))
    }
}

/// Map a browser **display name** (as resolved by the browser fast-path —
/// `"Google Chrome"`, `"Brave Browser"`, …) to the token the Windows shell
/// `start` verb resolves via the `App Paths` registry. `None` for browsers that
/// don't exist on Windows (Safari/Arc) or any unrecognized name, so the caller
/// falls back to the default URL handler. Matched case-insensitively by
/// substring so aliases ("Chrome", "Microsoft Edge") all resolve.
#[cfg(target_os = "windows")]
pub(crate) fn windows_browser_launch_token(app: &str) -> Option<&'static str> {
    let a = app.to_lowercase();
    // Order matters: check the more specific names first ("microsoft edge"
    // contains neither "chrome" nor "firefox", but keep edge before a bare
    // "chrome" check anyway for clarity).
    if a.contains("brave") {
        Some("brave")
    } else if a.contains("edge") {
        Some("msedge")
    } else if a.contains("firefox") {
        Some("firefox")
    } else if a.contains("chrome") || a.contains("chromium") {
        Some("chrome")
    } else {
        None
    }
}

/// Production backend: real AX primitives + a fast LLM for decisions.
pub struct RealBackend {
    config: crate::openhuman::config::Config,
}

impl RealBackend {
    pub fn new(config: crate::openhuman::config::Config) -> Self {
        Self { config }
    }
}

#[async_trait]
impl AutomateBackend for RealBackend {
    async fn perceive(&self, app: &str, filter: &str) -> Result<Vec<ax::AXElement>, String> {
        ax::ax_list_elements_filtered(app, filter)
    }

    async fn decide(&self, system: &str, user: &str) -> Result<String, String> {
        // Fast tier: the canonical `summarization` hint maps to `memory_provider`
        // (the Settings "Memory" routing row) — a cheap, quick model class, and on
        // the managed backend the dedicated `summarization-v1` tier. A dedicated
        // `automation` provider knob is a follow-up (see plan §5); routing through
        // `summarization` keeps M1 free of Config-schema churn while still keeping
        // the chat model out of the loop.
        let (provider, model) = crate::openhuman::inference::provider::create_chat_provider(
            "summarization",
            &self.config,
        )
        .map_err(|e| format!("fast-model provider unavailable: {e}"))?;
        provider
            .chat_with_system(Some(system), user, &model, 0.0)
            .await
            .map_err(|e| format!("fast-model call failed: {e}"))
    }

    async fn act_launch(&self, app: &str) -> Result<String, String> {
        crate::openhuman::tools::implementations::system::launch_platform(app).await
    }

    async fn act_press(&self, app: &str, label: &str) -> Result<String, String> {
        ax::ax_press_element(app, label)
    }

    async fn act_set_value(&self, app: &str, label: &str, value: &str) -> Result<String, String> {
        ax::ax_set_field_value(app, label, value)
    }

    async fn screenshot(
        &self,
        app: &str,
    ) -> Result<(String, super::vision_click::CaptureGeometry), String> {
        // Capture whatever window is frontmost — the loop guarantees the target
        // is frontmost before a vision_click, so this resolves to its window.
        let ctx = super::foreground_context()
            .ok_or_else(|| "could not resolve the foreground window for capture".to_string())?;
        if let Some(name) = ctx.app_name.as_deref() {
            if !name.eq_ignore_ascii_case(app) {
                log::warn!(
                    "{LOG_PREFIX} screenshot: frontmost {name:?} != target {app:?}; capturing frontmost"
                );
            }
        }
        // `capture_window_geometry` shells out to `screencapture` (blocking).
        match tokio::task::spawn_blocking(move || {
            super::vision_click::capture_window_geometry(&ctx)
        })
        .await
        {
            Ok(inner) => inner,
            Err(e) => Err(format!("capture task join failed: {e}")),
        }
    }

    async fn locate(
        &self,
        screenshot: &str,
        geom: &super::vision_click::CaptureGeometry,
        description: &str,
    ) -> Result<Option<(i32, i32)>, String> {
        // Use the main `chat` provider's vision model (per plan): reliable UI
        // grounding, and the fallback only fires when AX is empty (rare).
        let (provider, model) =
            crate::openhuman::inference::provider::create_chat_provider("chat", &self.config)
                .map_err(|e| format!("vision provider unavailable: {e}"))?;
        let coords =
            super::vision_click::locate_via_vision(&*provider, &model, screenshot, description)
                .await?;
        Ok(coords.map(|(px, py)| super::vision_click::image_to_screen(geom, px, py)))
    }

    async fn frontmost_app(&self) -> Option<String> {
        tokio::task::spawn_blocking(|| super::foreground_context().and_then(|c| c.app_name))
            .await
            .ok()
            .flatten()
    }

    async fn click(&self, x: i32, y: i32) -> Result<String, String> {
        super::vision_click::guarded_click(x, y).await
    }

    async fn open_url(&self, url: &str) -> Result<String, String> {
        // Cross-platform URI opener. macOS `open`, Linux `xdg-open`, Windows
        // `cmd /C start`. Only invoked by fast-paths with app-controlled URLs
        // (never user free-text), so there's no untrusted-URL surface here.
        #[cfg(target_os = "macos")]
        let mut cmd = {
            let mut c = tokio::process::Command::new("open");
            c.arg(url);
            c
        };
        #[cfg(target_os = "linux")]
        let mut cmd = {
            let mut c = tokio::process::Command::new("xdg-open");
            c.arg(url);
            c
        };
        #[cfg(target_os = "windows")]
        let mut cmd = {
            let mut c = tokio::process::Command::new("cmd");
            c.args(["/C", "start", "", url]);
            c
        };
        match cmd.output().await {
            Ok(o) if o.status.success() => Ok(format!("Opened {url}")),
            Ok(o) => Err(format!(
                "opener exited {}: {}",
                o.status,
                String::from_utf8_lossy(&o.stderr).trim()
            )),
            Err(e) => Err(format!("failed to launch opener: {e}")),
        }
    }

    async fn open_url_in_app(&self, app: &str, url: &str) -> Result<String, String> {
        // macOS: `open -a "<app>" "<url>"` both launches/foregrounds the named
        // app AND opens the URL in it — exactly the deterministic browser nav we
        // want (no address-bar typing, no AX).
        #[cfg(target_os = "macos")]
        {
            match tokio::process::Command::new("open")
                .arg("-a")
                .arg(app)
                .arg(url)
                .output()
                .await
            {
                Ok(o) if o.status.success() => Ok(format!("Opened {url} in {app}")),
                Ok(o) => Err(format!(
                    "open -a {app} exited {}: {}",
                    o.status,
                    String::from_utf8_lossy(&o.stderr).trim()
                )),
                Err(e) => Err(format!("failed to launch opener: {e}")),
            }
        }
        // Windows: the shell `start` verb resolves a browser by its registered
        // App Paths token (`chrome`, `msedge`, `firefox`, `brave`, …) and opens
        // the URL in it. When the browser is already running this lands in a NEW
        // TAB of the existing window — so the deterministic fast-path does NOT
        // pile up windows (the live bug: each re-delegation `launch_app`-ed Chrome
        // again → ~10 windows). Falls back to the default handler when the named
        // browser has no known token (e.g. Safari/Arc, which aren't on Windows).
        #[cfg(target_os = "windows")]
        {
            let Some(token) = windows_browser_launch_token(app) else {
                log::info!(
                    "[automate] open_url_in_app: no Windows token for {app:?}; using default handler"
                );
                return self.open_url(url).await;
            };
            // `cmd /C start "" <token> "<url>"` — the empty "" is `start`'s title
            // arg (required so a quoted token isn't mistaken for the title). The
            // URL is app-controlled (built by the fast-path), never user free-text.
            match tokio::process::Command::new("cmd")
                .args(["/C", "start", "", token, url])
                .output()
                .await
            {
                Ok(o) if o.status.success() => Ok(format!("Opened {url} in {app}")),
                Ok(o) => {
                    // `start` failed (token not registered?) — best-effort fall back.
                    log::warn!(
                        "[automate] open_url_in_app: start {token} exited {}: {}; falling back",
                        o.status,
                        String::from_utf8_lossy(&o.stderr).trim()
                    );
                    self.open_url(url).await
                }
                Err(e) => Err(format!("failed to launch opener: {e}")),
            }
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = app;
            self.open_url(url).await
        }
    }

    async fn key(&self, keys: &[String]) -> Result<String, String> {
        use crate::openhuman::tools::implementations::computer::keyboard;
        match keys.len() {
            0 => Err("no keys provided".to_string()),
            1 => keyboard::run_key(&keys[0]).await,
            _ => keyboard::run_hotkey(keys).await,
        }
    }

    async fn type_text(&self, text: &str) -> Result<String, String> {
        crate::openhuman::tools::implementations::computer::keyboard::run_type_text(text).await
    }

    async fn verify_playing(&self) -> Option<bool> {
        // macOS: ask Apple Music for ground-truth player state. Other OSes can't
        // verify this way → None (fast-path treats None as best-effort).
        #[cfg(target_os = "macos")]
        {
            let out = tokio::process::Command::new("osascript")
                .args(["-e", "tell application \"Music\" to player state as string"])
                .output()
                .await
                .ok()?;
            let state = String::from_utf8_lossy(&out.stdout).trim().to_lowercase();
            Some(state == "playing")
        }
        #[cfg(not(target_os = "macos"))]
        {
            None
        }
    }

    async fn now_playing(&self) -> Option<(String, String)> {
        // macOS: ask Apple Music for the current track's name + artist. We join
        // them with a tab (unlikely in titles) so we can split unambiguously.
        #[cfg(target_os = "macos")]
        {
            let script = "tell application \"Music\" to try
                set t to current track
                return (name of t) & \"\\t\" & (artist of t)
            end try";
            let out = tokio::process::Command::new("osascript")
                .args(["-e", script])
                .output()
                .await
                .ok()?;
            let line = String::from_utf8_lossy(&out.stdout);
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let (name, artist) = line.split_once('\t')?;
            Some((name.trim().to_string(), artist.trim().to_string()))
        }
        #[cfg(not(target_os = "macos"))]
        {
            None
        }
    }

    async fn settle(&self, app: &str) {
        // M2: poll the element count until the UI stops changing (≤2s), instead
        // of a blind fixed wait. Removes the timing-race class (tracker §1.11/
        // §1.13) — the next perceive sees a settled tree. `ax_wait_settled` is
        // blocking (synchronous helper IPC), so run it off the async runtime.
        let app = app.to_string();
        let _ = tokio::task::spawn_blocking(move || {
            ax::ax_wait_settled(&app, 240, 2000);
        })
        .await;
    }
}

#[cfg(test)]
#[path = "automate_tests.rs"]
mod tests;

#[cfg(all(test, target_os = "windows"))]
mod windows_tests {
    use super::windows_browser_launch_token as tok;

    #[test]
    fn browser_display_names_map_to_start_tokens() {
        assert_eq!(tok("Google Chrome"), Some("chrome"));
        assert_eq!(tok("Brave Browser"), Some("brave"));
        assert_eq!(tok("Microsoft Edge"), Some("msedge"));
        assert_eq!(tok("Firefox"), Some("firefox"));
        // Aliases / case-insensitive.
        assert_eq!(tok("chrome"), Some("chrome"));
        assert_eq!(tok("EDGE"), Some("msedge"));
        // Not on Windows / unknown → None → caller uses the default handler.
        assert_eq!(tok("Safari"), None);
        assert_eq!(tok("Arc"), None);
        assert_eq!(tok("Some Random App"), None);
    }
}
