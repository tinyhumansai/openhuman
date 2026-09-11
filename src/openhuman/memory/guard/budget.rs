//! Char-budget truncation — step 6 of the guard's enforcement chain.
//!
//! Pure functions over already-materialised values, with no policy, no config
//! and no I/O, so the budget arithmetic is testable without constructing a
//! provider.
//!
//! ## Chars, not bytes
//!
//! Every count here is [`str::chars`], never [`str::len`]. `len()` is a byte
//! count, so a budget expressed in "chars" would truncate a Japanese or emoji
//! transcript to a third of its stated size — and, worse, `String::truncate`
//! on a byte index panics when that index is not a char boundary. Slicing at a
//! char boundary is the only form that is both correct and total.

use crate::openhuman::memory::api::types::MemoryEntry;

/// Truncate `content` to at most `max_chars` characters.
///
/// Returns the input unchanged (and unallocated) when it already fits, so the
/// common case costs nothing.
pub fn truncate_content(content: &str, max_chars: usize) -> std::borrow::Cow<'_, str> {
    let mut chars = content.char_indices();
    match chars.nth(max_chars) {
        // Fewer than `max_chars + 1` chars ⇒ it fits.
        None => std::borrow::Cow::Borrowed(content),
        Some((byte_idx, _)) => std::borrow::Cow::Owned(content[..byte_idx].to_string()),
    }
}

/// Outcome of applying a recall budget to a result set.
#[derive(Debug, Clone)]
pub struct BudgetOutcome {
    /// Entries that survived, in input order. At most one of them has had its
    /// `content` shortened — the entry that straddles the budget boundary.
    pub entries: Vec<MemoryEntry>,
    /// How many entries were dropped whole because the budget was already
    /// spent when they were reached.
    pub dropped: usize,
    /// How many characters of content were removed in total, across the
    /// truncated entry and the dropped ones.
    pub trimmed_chars: usize,
}

/// Apply a **cumulative** char budget across a ranked recall result.
///
/// The budget is spent in rank order, which is what makes truncation
/// least-destructive: recall returns most-relevant-first, so the entries that
/// lose content are the ones the caller was least likely to use. An entry that
/// straddles the boundary is kept with its content shortened rather than
/// dropped, because dropping it would silently change the hit count a caller
/// may be reporting.
///
/// A zero-length budget is not special-cased here — the caller decides whether
/// `0` means "disabled" (it does; see `GuardPolicy::recall_budget`) before
/// calling.
pub fn truncate_entries(entries: Vec<MemoryEntry>, max_chars: usize) -> BudgetOutcome {
    let mut remaining = max_chars;
    let mut kept: Vec<MemoryEntry> = Vec::with_capacity(entries.len());
    let mut dropped = 0usize;
    let mut trimmed_chars = 0usize;

    for mut entry in entries {
        if remaining == 0 {
            trimmed_chars += entry.content.chars().count();
            dropped += 1;
            continue;
        }
        let len = entry.content.chars().count();
        if len <= remaining {
            remaining -= len;
            kept.push(entry);
        } else {
            trimmed_chars += len - remaining;
            entry.content = truncate_content(&entry.content, remaining).into_owned();
            remaining = 0;
            kept.push(entry);
        }
    }

    BudgetOutcome {
        entries: kept,
        dropped,
        trimmed_chars,
    }
}

#[cfg(test)]
#[path = "budget_tests.rs"]
mod tests;
