//! Deterministic per-app accelerators for the `automate` loop.
//!
//! A fast-path encodes a *proven* native sequence for a common (app, intent)
//! pair so the loop doesn't have to rediscover it with the model every time.
//! [`try_fastpath`] is consulted **before** the general loop and returns:
//!   - `Some(success)`  → the loop returns it directly,
//!   - `Some(failure)`  → the loop logs and falls through to the model loop,
//!   - `None`           → no fast-path applies; straight to the model loop.
//!
//! So a fast-path can only *help*. This is deliberately different from the
//! removed `play_music` tool (tracker §1.13): that was a separate tool the LLM
//! had to choose (and chose wrong); this is internal to `automate`, transparent,
//! and always backed by the general loop.

use super::automate::AutomateBackend;
use super::automate::AutomateOutcome;

mod app_shortcuts;
mod browser;
mod browser_shortcuts;
mod music;

/// Try every registered fast-path; return the first that claims the (app, goal).
pub async fn try_fastpath(
    app: &str,
    goal: &str,
    backend: &dyn AutomateBackend,
) -> Option<AutomateOutcome> {
    if music::matches(app, goal) {
        return Some(music::run(goal, backend).await);
    }
    if browser::matches(app, goal) {
        return Some(browser::run(app, goal, backend).await);
    }
    if app_shortcuts::matches(app, goal) {
        return Some(app_shortcuts::run(app, goal, backend).await);
    }
    None
}

#[cfg(test)]
#[path = "fastpaths_tests.rs"]
mod tests;
