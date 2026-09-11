//! Which billing route a recorded cost belongs to (issue #5016).
//!
//! The local `[cost]` daily/monthly limits exist to cap spend against
//! **OpenHuman-managed credits**. They were being enforced against *every*
//! recorded call, including bring-your-own-key (BYOK) and local inference that
//! OpenHuman never bills for. A BYOK user therefore accumulated phantom spend
//! — priced locally from [`super::catalog`] because their provider echoes no
//! `charged_amount_usd` — until they tripped the default $10/day cap and got
//! "You're out of credits", despite OpenHuman having charged them nothing.
//!
//! The route is derived from the recorded model id rather than threaded
//! through as a new parameter, which matters for two reasons:
//!
//! 1. Every recording site already has the model id, so no call site has to
//!    learn about routing.
//! 2. Records persisted by older builds carry a model id too, so history
//!    classifies correctly with **no migration and no schema change** — a
//!    stored-route field would have had to guess a default for every existing
//!    record and would keep mis-gating real users for up to a month.

/// The billing route a cost record belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CostRoute {
    /// Served by the OpenHuman managed backend and paid for with OpenHuman
    /// credits. Counts toward — and is gated by — the local `[cost]` limits.
    Managed,
    /// Served by the user's own key (OpenRouter, Anthropic, a self-hosted
    /// OpenAI-compatible gateway, …) or a local model. OpenHuman bills nothing
    /// for it, so it is recorded for the dashboard but never gated.
    Byok,
}

impl CostRoute {
    /// Whether spend on this route counts toward the local `[cost]` budget.
    pub fn counts_toward_budget(self) -> bool {
        matches!(self, CostRoute::Managed)
    }
}

/// Model ids the managed backend serves. These are the backend's own tier
/// slugs (`crate::openhuman::config::MODEL_*_V1`) — a BYOK
/// provider is always addressed by its real model id (`minimax/minimax-m3`,
/// `anthropic/claude-sonnet-4-20250514`, `llama3:8b`), never by a tier slug,
/// because the tier vocabulary only means something to the managed backend.
///
/// Kept as a local list, deliberately: this is the set of ids that imply
/// *managed billing*, which is a narrower question than "is this a known tier
/// constant". A new tier must be added here consciously, and the
/// `managed_tier_slugs_stay_in_sync` test fails if one is added upstream
/// without doing so.
const MANAGED_MODEL_SLUGS: &[&str] = &[
    "chat-v1",
    "reasoning-v1",
    "reasoning-quick-v1",
    "agentic-v1",
    "burst-v1",
    "coding-v1",
    "vision-v1",
    "summarization-v1",
];

/// Classify a recorded model id into its billing route.
///
/// Unknown ids classify as [`CostRoute::Byok`], which is the safe direction:
/// the failure mode is under-enforcing a *local* convenience cap, not
/// over-charging. Real managed-credit exhaustion is still enforced
/// server-side, where the backend returns its own out-of-credits error — that
/// path is untouched by this classification.
pub fn route_for_model(model: &str) -> CostRoute {
    let normalized = normalize_model_id(model);
    if MANAGED_MODEL_SLUGS.contains(&normalized.as_str()) || is_managed_passthrough_id(&normalized)
    {
        CostRoute::Managed
    } else {
        CostRoute::Byok
    }
}

/// `openrouter/<author>/<slug>` ids served by the managed backend's OpenRouter
/// passthrough (`OPENROUTER_PASSTHROUGH_ENABLED`).
///
/// These are **managed** spend even though they are not tier slugs: the request
/// goes to our backend, is billed against managed credits, and never touches a
/// user-held key. Without this they fell to the `Byok` default and silently
/// stopped counting toward the local managed cap — the same failure the
/// `hint:` / `openhuman/` decoration stripping in [`normalize_model_id`] exists
/// to prevent, reached by a different route.
///
/// A BYOK OpenRouter provider addresses models by their bare upstream slug
/// (`deepseek/deepseek-v4-flash`), never with this `openrouter/` qualifier, so
/// the two cannot be confused.
fn is_managed_passthrough_id(normalized: &str) -> bool {
    let Some(rest) = normalized.strip_prefix("openrouter/") else {
        return false;
    };
    // EXACTLY two non-empty segments. Filtering empties before counting would
    // accept `openrouter/a//b` and `openrouter/a/b/`, which are not the
    // passthrough shape; classifying a malformed id as managed would count it
    // toward the managed cap on the strength of a typo.
    let mut segments = rest.split('/');
    match (segments.next(), segments.next(), segments.next()) {
        (Some(author), Some(slug), None) => !author.is_empty() && !slug.is_empty(),
        _ => false,
    }
}

/// Lower-case, trim, and strip the decorations a model id can pick up on its
/// way to a cost record: a `hint:` prefix from the tier-resolution helpers and
/// an `openhuman/` provider qualifier.
///
/// Stripping loops until neither prefix applies, so the two decorations are
/// **order-independent**. A single fixed-order pass classified
/// `openhuman/hint:chat-v1` as BYOK (it stripped `openhuman/`, leaving
/// `hint:chat-v1`, which is not a managed slug) while `hint:openhuman/chat-v1`
/// classified as Managed — meaning managed spend could silently stop counting
/// toward the cap depending only on which decoration a recording site applied
/// first.
fn normalize_model_id(model: &str) -> String {
    let mut current = model.trim().to_ascii_lowercase();
    loop {
        let stripped = current
            .strip_prefix("hint:")
            .or_else(|| current.strip_prefix("openhuman/"));
        match stripped {
            Some(rest) => current = rest.to_string(),
            None => return current,
        }
    }
}

#[cfg(test)]
#[path = "route_tests.rs"]
mod tests;
