//! Ranked, normalized matching of accessibility elements against a target label.
//!
//! Element selection today is a naive case-insensitive substring `contains` that
//! takes the *first* hit in tree order: `ax_list_elements_filtered` filters that
//! way, and the Swift helper's press path matches "exact-first, else the first
//! `contains`". That is unreliable for the "reliable UI element clicking" #3202
//! calls out, in two ways:
//!
//!   1. **Ambiguity** — several controls share a substring. Filtering for `Save`
//!      also surfaces `Save As…` and `Autosave`, and "first in tree order" is not
//!      the same as "the control the user meant".
//!   2. **Near-misses** — the model asks for a label that is *almost* right:
//!      different case, surrounding whitespace, a trailing `…` on a menu item, or
//!      an extra space the AX label collapses. A literal `contains` misses these
//!      even though a human reads them as the same target.
//!
//! This module ranks candidates by match quality ([`MatchTier`]) so the best,
//! least-ambiguous element sorts first. `ax_list_elements_filtered` uses
//! [`filter_and_rank`] to order (and, with the tool's top-N cap, *keep*) the best
//! matches; [`best_match`] additionally reports whether the top match is
//! ambiguous, the reusable primitive a later press-disambiguation slice can use
//! to ask the user which control they meant instead of clicking the wrong one.
//!
//! Everything here is pure — no AX/FFI, no clock — so it is exhaustively
//! unit-tested and shared verbatim by every backend.

use super::ax_interact::AXElement;

/// How well a candidate label matches a requested label, best (`Exact`) to worst
/// (`Substring`). The derived `Ord` orders variants by declaration, so an
/// ascending sort places the strongest match first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchTier {
    /// Byte-for-byte identical.
    Exact,
    /// Equal ignoring ASCII case (`Save` vs `save`).
    CaseInsensitiveExact,
    /// Equal after normalization — case-folded, whitespace-collapsed, trailing
    /// ellipsis stripped (`Save As…` vs `save as`).
    NormalizedExact,
    /// Normalized candidate starts with the normalized query (`Save As…` for
    /// query `save`).
    Prefix,
    /// Normalized query appears at a word boundary inside the candidate
    /// (`Discard changes` for query `changes`).
    WordBoundary,
    /// Normalized query appears somewhere inside the candidate (weakest signal).
    Substring,
}

/// The chosen element plus why it was chosen and whether the choice was
/// contested. `ambiguous` is `true` when another candidate tied at the same
/// [`MatchTier`] with a *different* normalized label — i.e. the query alone
/// cannot say which control the user meant.
#[derive(Debug, Clone, Copy)]
pub struct ElementMatch<'a> {
    pub element: &'a AXElement,
    pub tier: MatchTier,
    pub ambiguous: bool,
}

/// Fold a label into its comparable form: trimmed, lowercased, trailing ellipsis
/// (`…` or `...`) removed, and internal whitespace runs collapsed to one space.
///
/// This is deliberately conservative — it never strips role words like `button`
/// (which would create false matches), only the incidental differences a human
/// reads through.
pub(crate) fn normalize(s: &str) -> String {
    let mut t = s.trim().to_lowercase();
    // Strip any run of trailing ellipsis markers (menu items commonly end in one),
    // re-trimming trailing space between passes so "Save …" reduces cleanly.
    loop {
        let trimmed = t.trim_end();
        if let Some(stripped) = trimmed
            .strip_suffix('…')
            .or_else(|| trimmed.strip_suffix("..."))
        {
            t = stripped.to_string();
        } else {
            t = trimmed.to_string();
            break;
        }
    }
    // Collapse internal whitespace and drop leading/trailing space in one pass.
    t.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `true` when `needle` occurs in `haystack` as a whole word run — delimited by
/// the string edge or a non-alphanumeric char on **both** sides. Both inputs are
/// expected already normalized.
///
/// Both boundaries matter: a leading-only check would let `changes` match inside
/// `changeset` (partial word). Requiring a trailing boundary too keeps
/// [`MatchTier::WordBoundary`] to genuine word matches; partial-word hits fall
/// through to the weaker [`MatchTier::Substring`].
fn is_word_boundary_match(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    haystack.match_indices(needle).any(|(i, m)| {
        let before_ok = haystack[..i]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric());
        let after_ok = haystack[i + m.len()..]
            .chars()
            .next()
            .is_none_or(|c| !c.is_alphanumeric());
        before_ok && after_ok
    })
}

/// Classify how `candidate` matches `query`, or `None` if it does not match at
/// all. A blank query matches nothing (an empty needle would otherwise match
/// every control).
pub(crate) fn classify(candidate: &str, query: &str) -> Option<MatchTier> {
    if query.trim().is_empty() {
        return None;
    }
    if candidate == query {
        return Some(MatchTier::Exact);
    }
    if candidate.eq_ignore_ascii_case(query) {
        return Some(MatchTier::CaseInsensitiveExact);
    }
    let nc = normalize(candidate);
    let nq = normalize(query);
    if nq.is_empty() {
        return None;
    }
    if nc == nq {
        return Some(MatchTier::NormalizedExact);
    }
    if nc.starts_with(&nq) {
        return Some(MatchTier::Prefix);
    }
    if is_word_boundary_match(&nc, &nq) {
        return Some(MatchTier::WordBoundary);
    }
    if nc.contains(&nq) {
        return Some(MatchTier::Substring);
    }
    None
}

/// Keep only the elements that match `query` and return them best-match-first.
///
/// Membership is the same set a case-insensitive `contains` would keep (plus the
/// few near-misses normalization reconciles); the win is ordering — with the
/// tool's fixed top-N render cap, "best first" means the cap keeps the *best* N
/// rather than an arbitrary N. Ordering is stable: candidates in the same tier
/// keep their original tree order, so the result is deterministic.
pub(crate) fn filter_and_rank(elements: Vec<AXElement>, query: &str) -> Vec<AXElement> {
    let mut ranked: Vec<(MatchTier, usize, AXElement)> = elements
        .into_iter()
        .enumerate()
        .filter_map(|(idx, el)| classify(&el.label, query).map(|tier| (tier, idx, el)))
        .collect();
    ranked.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    ranked.into_iter().map(|(_, _, el)| el).collect()
}

/// Pick the single best element for `query`, reporting its [`MatchTier`] and
/// whether the choice was ambiguous. `None` when nothing matches.
pub fn best_match<'a>(elements: &'a [AXElement], query: &str) -> Option<ElementMatch<'a>> {
    let mut ranked: Vec<(MatchTier, usize, &'a AXElement)> = elements
        .iter()
        .enumerate()
        .filter_map(|(idx, el)| classify(&el.label, query).map(|tier| (tier, idx, el)))
        .collect();
    ranked.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    let &(best_tier, _, best_el) = ranked.first()?;
    let best_norm = normalize(&best_el.label);
    // Contested only when a same-tier rival names a *different* control; two
    // candidates whose labels normalize identically are the same target.
    let ambiguous = ranked
        .iter()
        .filter(|(tier, _, _)| *tier == best_tier)
        .any(|(_, _, el)| normalize(&el.label) != best_norm);
    Some(ElementMatch {
        element: best_el,
        tier: best_tier,
        ambiguous,
    })
}

#[cfg(test)]
#[path = "element_match_tests.rs"]
mod tests;
