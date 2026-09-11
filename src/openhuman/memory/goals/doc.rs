//! The validating mutation surface for [`GoalsDoc`] — add, edit, delete.
//!
//! # Why this is the host's and not the driver's (#5560)
//!
//! [`MemoryGoals`](crate::openhuman::memory::api::provider::MemoryGoals) has
//! exactly two members: read the whole document and replace the whole document.
//! That is deliberate, and its own contract docs say why —
//!
//! > Whole-document replacement rather than per-item add/edit/delete because
//! > the validating mutation surface (PII and secret predicates) is **host**
//! > policy: the host parses, validates, mutates, and hands back the result.
//! > Exposing per-item mutation here would put that policy behind a trait a
//! > third-party driver implements, where it could be skipped.
//!
//! So the trio below came home from `tinycortex::memory::goals::store`'s
//! `GoalsDocMutations` trait, and the predicates they call came with them —
//! except that they did not have to travel, because
//! [`safety`](crate::openhuman::memory::safety) is where they started.
//! TinyCortex's `memory::store::safety` says so in its own first line ("Ported
//! from OpenHuman's `memory_store::safety`"), and the four functions
//! [`validate_goal_text`] needs — `has_likely_email`, `has_likely_pii`,
//! `has_likely_secret`, `sanitize_text` — are the originals, in this crate,
//! already. Nothing here is a second implementation of anything.
//!
//! ## Validation still runs twice, exactly as it did before
//!
//! The engine's `goals::store::save` re-validates every item before it writes,
//! and that has not changed: it is reached through
//! `MemoryGoals::set_goals`. So a document that gets past this module is
//! checked again at the one choke point that touches the file, and a driver
//! handed a bad document by any other route still refuses it. What this module
//! restores is the *first* check — the one that used to live in
//! `GoalsDocMutations` and produce the specific message a caller acts on
//! ("goal text must be a single line") rather than `save`'s catch-all
//! ("goal '<id>' text must be a non-empty, single-line, secret/PII-free
//! string").
//!
//! ## The mutation lock came home too, and it has to
//!
//! Every mutation is a read-modify-write over a whole document: `goals()`,
//! mutate here, `set_goals()`. The engine serialised that sequence behind a
//! process-wide `parking_lot::Mutex` (`goals_mutation_lock`) precisely because
//! two interleaved sequences lose one of the two writes outright — the second
//! `set_goals` replaces the document the first one wrote. Splitting the
//! sequence across two driver calls does not make that hazard smaller; it makes
//! the window wider. [`mutation_lock`] is the same lock on this side of the
//! boundary, and `ops` holds it across both calls.
//!
//! It is a `tokio::sync::Mutex` rather than a `parking_lot` one because the
//! guarded region now contains two `await` points. A `std`/`parking_lot` guard
//! held across an await is not `Send`, and blocking the executor thread for the
//! duration of two bus round-trips is the alternative nobody wants.

use tokio::sync::Mutex;

use crate::openhuman::memory::api::error::MemoryError;
use crate::openhuman::memory::api::goals::{GoalItem, GoalsDoc};
use crate::openhuman::memory::safety::{
    has_likely_email, has_likely_pii, has_likely_secret, sanitize_text,
};

/// Serialises `goals()` → mutate → `set_goals()` sequences across all host
/// callers. See the module docs for why the sequence needs one.
pub(super) fn mutation_lock() -> &'static Mutex<()> {
    static LOCK: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Detect likely PII in goal text (email addresses, generic PII patterns, or
/// content the sanitizer would otherwise redact).
///
/// The three-way test is the engine's `has_goal_pii`, unchanged: the boundary
/// predicate [`has_likely_pii`] is deliberately *strict* (it ignores
/// bare-numeric shapes to avoid rejecting scanner-built identifiers), so the
/// `sanitize_text` arm is what catches the rest — a goal is free prose, not an
/// identifier, and the strict set alone would let a phone number through.
fn has_goal_pii(text: &str) -> bool {
    has_likely_email(text) || has_likely_pii(text) || sanitize_text(text).report.pii_redactions > 0
}

/// Validate that `text` is a non-empty, single-line, secret- and PII-free goal
/// body, returning the trimmed text.
///
/// A newline-bearing goal would inject extra `- [..]` list lines on reload,
/// corrupting the stored shape — so it is rejected outright rather than
/// escaped.
///
/// # Errors
///
/// [`MemoryError::Invalid`] with the same three messages the engine's
/// `validate_goal_text` produced; they reach the agent as tool-result text and
/// are what it acts on to retry.
fn validate_goal_text(text: &str) -> Result<&str, MemoryError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(MemoryError::Invalid(
            "goal text must not be empty".to_string(),
        ));
    }
    if text.contains('\n') || text.contains('\r') {
        return Err(MemoryError::Invalid(
            "goal text must be a single line".to_string(),
        ));
    }
    if has_likely_secret(text) || has_goal_pii(text) {
        return Err(MemoryError::Invalid(
            "goal text must not contain secrets or PII".to_string(),
        ));
    }
    Ok(text)
}

/// Append a new goal to `doc`, returning the assigned id.
///
/// In-memory only — pair with `MemoryGoals::set_goals`. The id comes from
/// [`GoalsDoc::next_id`], which is the contract's own allocator, so the
/// `g<N>` sequence is unchanged.
///
/// # Errors
///
/// As [`validate_goal_text`].
pub(super) fn add_item(doc: &mut GoalsDoc, text: &str) -> Result<String, MemoryError> {
    let text = validate_goal_text(text)?;
    let id = doc.next_id();
    doc.items.push(GoalItem::new(&id, text));
    Ok(id)
}

/// Replace the text of the goal with `id`.
///
/// # Errors
///
/// [`MemoryError::NotFound`] for an unknown id, otherwise as
/// [`validate_goal_text`].
pub(super) fn edit_item(doc: &mut GoalsDoc, id: &str, text: &str) -> Result<(), MemoryError> {
    let text = validate_goal_text(text)?;
    let item = doc
        .items
        .iter_mut()
        .find(|i| i.id == id)
        .ok_or_else(|| MemoryError::NotFound(format!("no goal with id '{id}'")))?;
    item.text = text.to_string();
    Ok(())
}

/// Delete the goal with `id`.
///
/// `position` + `remove` rather than `retain`, which would drop every item
/// sharing `id`. A well-formed document never has duplicate ids, but
/// [`GoalsDoc::parse`] accepts a hand-edited or corrupt file that does — this
/// matches [`edit_item`]'s "first occurrence" semantics so such a document
/// loses at most one goal per delete rather than all of them.
///
/// # Errors
///
/// [`MemoryError::NotFound`] for an unknown id.
pub(super) fn delete_item(doc: &mut GoalsDoc, id: &str) -> Result<(), MemoryError> {
    let index = doc
        .items
        .iter()
        .position(|item| item.id == id)
        .ok_or_else(|| MemoryError::NotFound(format!("no goal with id '{id}'")))?;
    doc.items.remove(index);
    Ok(())
}

#[cfg(test)]
#[path = "doc_tests.rs"]
mod tests;
