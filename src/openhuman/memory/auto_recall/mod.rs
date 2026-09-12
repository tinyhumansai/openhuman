//! Lane C — a gated, bounded pre-turn recall of facts about the user (#6040).
//!
//! The chat turn assembles a small per-turn context that rides on the user
//! message, never on the cached system-prompt prefix. Lane B, situational
//! preferences, has always been there. This lane sits beside it and answers the
//! one question the model could not answer on its own: *does memory hold
//! something about the user that this message is asking for?* Without it,
//! "who is my idol and why?" was answered from prompt history alone, and the
//! model — never told to look — replied "not stored" while the fact sat in the
//! memory tree.
//!
//! # Why this is not the lane that was removed
//!
//! `core_turn.rs` documents the old `memory_loader.load_context()` block: two
//! full scans of the `global` namespace on every turn, for at most nine lines,
//! most of them empty once ordinary chat crowded the ranking. This lane is a
//! different cost class on every axis that mattered there:
//!
//! - **It runs on a minority of turns.** [`gate_decision`] opens only for a
//!   message that asks about the user — first-person ownership plus a question
//!   or request shape. Small talk, code, weather and pastes never reach the
//!   store.
//! - **It walks the tree, not the pile — and reads the notes.** [`GuardSource`]
//!   calls the bound driver's `fast_retrieve` through the guard: summary-first,
//!   and on an entity-less query the engine's dense path, with a limit of
//!   [`AUTO_RECALL_LIMIT`]. Beside it runs one scored recall over the
//!   assistant's own namespace, [`AUTO_RECALL_NOTES_NAMESPACE`] (#6063): the
//!   store `memory_store` writes to and the tree never sees, so a fact the user
//!   asked to keep seconds ago is found where it was filed. Each leg is bounded
//!   by the answer, not by everything the user ever said — the notes leg is one
//!   query embed against a namespace of explicit notes, not the two full scans
//!   the removed lane paid, and it runs only on the gated turns.
//! - **It is floored, capped and budgeted.** Tree hits below
//!   [`AUTO_RECALL_RELATIVE_FLOOR`] of the best score are dropped (retrieval
//!   scores are declared non-comparable across drivers, so the floor is
//!   relative, not absolute); notes below [`AUTO_RECALL_NOTE_MIN_SIMILARITY`]
//!   are dropped (the namespace recall reports the cosine component on its own,
//!   so that floor is absolute, and measured); at most [`AUTO_RECALL_LIMIT`]
//!   survive per leg, each clipped to [`AUTO_RECALL_PER_HIT_CHARS`]; lines that
//!   would push the block past the guard's `recall_max_chars` are left out
//!   whole, so no marker is ever cut; and each leg is abandoned after
//!   [`AUTO_RECALL_BUDGET`] on its own, so a memory module still downloading on
//!   a cold launch cannot stall the turn, and a slow tree cannot cost the notes
//!   their answer.
//!
//! # The hint
//!
//! The block opens with [`AUTO_RECALL_HINT`]: it is a pre-fetch, not a search.
//! Without that line the model read the block as the retrieval the memory-access
//! instruction demands and, handed three unrelated lines, declared a fact "not
//! on record" while the store held it (#6063).
//!
//! # Provenance
//!
//! A hit from a source tree — email, Slack, Notion, a web page, a folder — is
//! third-party content that an author can fill with instructions, so it is
//! rendered inside the `<untrusted-source>` marker the older recall path used,
//! with the scope prefix as the hint. Chat-tree hits, which the user or the
//! assistant wrote, are rendered bare. A note is the user's or the assistant's
//! own words filed by `memory_store`, rendered bare too — unless it carries the
//! `ExternalSync` taint or a connector-prefixed key, the rule
//! `memory_context_safety` already applies to the older recall path, in which
//! case it is wrapped like a source hit.
//!
//! # Switch
//!
//! `[subsystems.memory.hooks] auto_recall` (env
//! `OPENHUMAN_MEMORY_HOOKS_AUTO_RECALL`) is the kill-switch. The flag existed
//! before this lane — declared, parsed, and read by nothing — so turning the
//! lane off is a config edit on any install, with no new key to learn.
//!
//! # Failure mode
//!
//! Every failure is an ordinary turn without a block: a driver without the
//! retrieval family, a retrieval error, a budget timeout. None of them is an
//! error to the user, and each leaves an `[auto_recall]` line behind so the
//! thresholds can be tuned from real logs.

mod gate;
mod source;
pub mod warm;

pub use gate::{gate_decision, GateDecision};
pub use source::{AutoRecallSource, GuardSource};

use crate::openhuman::agent::harness::memory_context_safety::{
    is_potentially_untrusted, wrap_untrusted_for_agent,
};
use crate::openhuman::agent::tinyagents::host::agent_memory::DEFAULT_AGENT_MEMORY_NAMESPACE;
use crate::openhuman::memory::api::provider::retrieval::{FastRetrieveQuery, RetrievalHit};
use crate::openhuman::memory::api::types::{MemoryTaint, NamespaceMemoryHit};
use crate::openhuman::memory::guard::MemoryGuard;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Most hits a block may carry. Three is enough to answer a question about the
/// user and small enough that a wrong guess costs a few lines, not a screen.
pub const AUTO_RECALL_LIMIT: usize = 3;

/// Characters kept per hit. A tree leaf is a chunk, not a sentence; the model
/// needs the fact, not the whole page it came from.
pub const AUTO_RECALL_PER_HIT_CHARS: usize = 400;

/// Characters kept of a hit's scope label (`folder:profile`, `slack:#eng`).
pub const AUTO_RECALL_SCOPE_CHARS: usize = 80;

/// How long the turn waits for the lookup before proceeding without it.
///
/// Wider than Lane B's 3 s on purpose. A warm `fast_retrieve` through the
/// module measured 2–3 s on a loaded 16 GB desktop (field test, 2026-09-05),
/// and at 3 s one of three real questions lost its block to the timeout — the
/// turn then answers "not stored" for a fact that is stored, which is the bug
/// this lane exists to fix. The lane only runs on gated turns, and the two
/// lanes run side by side, so the worst case a turn pays is this bound alone.
pub const AUTO_RECALL_BUDGET: Duration = Duration::from_secs(5);

/// A hit is kept only while its score is at least this fraction of the best
/// hit's. Scores are "higher is better" and nothing more across drivers, so an
/// absolute threshold would be a guess; a relative one drops the long tail of
/// weak matches without pretending to know the scale.
pub const AUTO_RECALL_RELATIVE_FLOOR: f32 = 0.5;

/// The banner that heads the injected block. Tests and the prompt snapshot
/// look for it; the model reads it as the section title.
pub const AUTO_RECALL_BANNER: &str = "## Relevant memory for this message";

/// The line under the banner. It says what the block is — a bounded
/// pre-fetch — so the model does not mistake it for the retrieval the
/// memory-access instruction asks for before claiming absence (#6063).
pub const AUTO_RECALL_HINT: &str = "Pre-fetched from memory, not a search: if the answer is \
not here, search memory (`memory_recall`) before saying something is not stored.";

/// The namespace the notes leg reads: the assistant's own memory, where
/// `memory_store` files a fact the user asked it to keep (#6063).
pub const AUTO_RECALL_NOTES_NAMESPACE: &str = DEFAULT_AGENT_MEMORY_NAMESPACE;

/// A note is kept only when its vector similarity to the message clears this.
///
/// Absolute, unlike the tree floor: the namespace recall reports the cosine
/// component on its own, and the managed embedder's noise floor is measured —
/// unrelated message↔note pairs score 0.28–0.33, a coffee preference against
/// a café question 0.575 (Lane B's `[pref_recall]` data). Same value as Lane
/// B's `SITUATIONAL_MIN_SIMILARITY`, declared apart so each lane tunes alone.
/// The `[auto_recall]` line logs the best candidate before the floor.
pub const AUTO_RECALL_NOTE_MIN_SIMILARITY: f64 = 0.35;

/// The lane itself: a retrieval source, the switch, and the budgets.
pub struct AutoRecall {
    source: Arc<dyn AutoRecallSource>,
    enabled: bool,
    recall_max_chars: Option<usize>,
    budget: Duration,
}

impl AutoRecall {
    /// A lane over `source`. `enabled = false` makes [`Self::block_for`]
    /// answer `None` without touching the source; `recall_max_chars` clips the
    /// rendered block (the guard's `recall_max_chars` budget).
    pub fn new(
        source: Arc<dyn AutoRecallSource>,
        enabled: bool,
        recall_max_chars: Option<usize>,
    ) -> Self {
        Self {
            source,
            enabled,
            recall_max_chars,
            budget: AUTO_RECALL_BUDGET,
        }
    }

    /// The production lane: reads through `guard`, switched by the guard
    /// policy's `hooks.auto_recall`, clipped by its `recall_max_chars`.
    pub fn from_guard(guard: Arc<MemoryGuard>) -> Self {
        let (enabled, recall_max_chars) = {
            let policy = guard.policy();
            (policy.hooks().auto_recall, policy.recall_budget())
        };
        log::debug!(
            "[auto_recall] lane bound driver={} enabled={enabled} recall_max_chars={recall_max_chars:?}",
            guard.policy().driver_id()
        );
        Self::new(Arc::new(GuardSource::new(guard)), enabled, recall_max_chars)
    }

    /// Overrides the lookup budget. Tests use it to make the timeout path
    /// reachable without sleeping for seconds.
    #[must_use]
    pub fn with_budget(mut self, budget: Duration) -> Self {
        self.budget = budget;
        self
    }

    /// Whether the lane will run at all.
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// The block to prepend to `user_message`, or `None` when the lane is off,
    /// the gate is closed, or nothing relevant came back from either leg.
    /// Never an error: a turn without a block is an ordinary turn.
    ///
    /// Two legs run side by side (#6063): the tree walk, and the scored recall
    /// over the assistant's own namespace, where `memory_store` files what the
    /// user asked to keep. Each is bounded by the budget on its own, so a slow
    /// tree cannot cost the notes their answer or the other way round; the turn
    /// pays the slower leg, never the sum.
    pub async fn block_for(&self, user_message: &str) -> Option<String> {
        if !self.enabled {
            log::debug!("[auto_recall] disabled by hooks.auto_recall; skipping");
            return None;
        }
        let decision = gate_decision(user_message);
        let GateDecision::Open(reason) = decision else {
            log::debug!(
                "[auto_recall] gate=closed reason={} chars={}",
                decision.reason(),
                user_message.chars().count()
            );
            return None;
        };

        let started = Instant::now();
        let (tree, notes) = tokio::join!(
            self.tree_leg(user_message, reason),
            self.notes_leg(user_message, reason)
        );
        let notes_top = similarity_label(notes.top);
        if tree.hits.is_empty() && notes.hits.is_empty() {
            log::info!(
                "[auto_recall] gate=open reason={reason} hits=0 total={} notes=0 \
                 notes_top={notes_top} tree_ms={} notes_ms={} elapsed_ms={}",
                tree.total,
                tree.elapsed_ms,
                notes.elapsed_ms,
                started.elapsed().as_millis()
            );
            return None;
        }
        let block = render_block(&notes.hits, &tree.hits, self.recall_max_chars);
        log::info!(
            "[auto_recall] gate=open reason={reason} hits={} total={} notes={} \
             notes_top={notes_top} tree_ms={} notes_ms={} elapsed_ms={} chars={}",
            tree.hits.len(),
            tree.total,
            notes.hits.len(),
            tree.elapsed_ms,
            notes.elapsed_ms,
            started.elapsed().as_millis(),
            block.chars().count()
        );
        Some(block)
    }

    /// The tree leg: `fast_retrieve` over the memory tree, ranked and floored
    /// by [`select_hits`]. An error or the budget yields no hits and one warn
    /// line; the other leg is not affected.
    async fn tree_leg(&self, user_message: &str, reason: &str) -> TreeLeg {
        let started = Instant::now();
        let query = FastRetrieveQuery {
            limit: AUTO_RECALL_LIMIT,
            ..FastRetrieveQuery::default()
        };
        let mut leg = TreeLeg::default();
        match tokio::time::timeout(self.budget, self.source.fast_retrieve(user_message, query))
            .await
        {
            Ok(Ok(response)) => {
                leg.total = response.total;
                leg.hits = select_hits(response.hits);
            }
            Ok(Err(err)) => log::warn!(
                "[auto_recall] gate=open reason={reason} tree retrieval failed after {}ms; \
                 continuing without tree hits: {err}",
                started.elapsed().as_millis()
            ),
            Err(_elapsed) => log::warn!(
                "[auto_recall] gate=open reason={reason} tree retrieval exceeded {:?}; \
                 continuing without tree hits",
                self.budget
            ),
        }
        leg.elapsed_ms = started.elapsed().as_millis();
        leg
    }

    /// The notes leg: the scored recall over [`AUTO_RECALL_NOTES_NAMESPACE`],
    /// floored by [`select_notes`]. Same footing as the tree leg on an error or
    /// the budget.
    async fn notes_leg(&self, user_message: &str, reason: &str) -> NotesLeg {
        let started = Instant::now();
        let mut leg = NotesLeg::default();
        match tokio::time::timeout(
            self.budget,
            self.source.recall_namespace_scored(
                AUTO_RECALL_NOTES_NAMESPACE,
                user_message,
                AUTO_RECALL_LIMIT,
            ),
        )
        .await
        {
            Ok(Ok(candidates)) => {
                leg.top = top_similarity(&candidates);
                leg.hits = select_notes(candidates);
            }
            Ok(Err(err)) => log::warn!(
                "[auto_recall] gate=open reason={reason} notes recall failed after {}ms; \
                 continuing without notes: {err}",
                started.elapsed().as_millis()
            ),
            Err(_elapsed) => log::warn!(
                "[auto_recall] gate=open reason={reason} notes recall exceeded {:?}; \
                 continuing without notes",
                self.budget
            ),
        }
        leg.elapsed_ms = started.elapsed().as_millis();
        leg
    }
}

/// What the tree leg answered: the hits that survived [`select_hits`], the
/// engine's pre-truncation total, and how long the lookup took.
#[derive(Default)]
struct TreeLeg {
    hits: Vec<RetrievalHit>,
    total: usize,
    elapsed_ms: u128,
}

/// What the notes leg answered. `top` is the best vector similarity among the
/// candidates *before* the floor — the number the floor is tuned from — or
/// `NEG_INFINITY` when the store offered none.
struct NotesLeg {
    hits: Vec<NamespaceMemoryHit>,
    top: f64,
    elapsed_ms: u128,
}

impl Default for NotesLeg {
    fn default() -> Self {
        Self {
            hits: Vec::new(),
            top: f64::NEG_INFINITY,
            elapsed_ms: 0,
        }
    }
}

/// The best vector similarity among `notes`, or `NEG_INFINITY` when there is
/// none finite to report.
pub(crate) fn top_similarity(notes: &[NamespaceMemoryHit]) -> f64 {
    notes
        .iter()
        .map(|note| note.score_breakdown.vector_similarity)
        .filter(|similarity| similarity.is_finite())
        .fold(f64::NEG_INFINITY, f64::max)
}

/// `top` for the log line: three decimals, or `none` when the store offered
/// no candidate at all — the two cases the floor is tuned apart on.
pub(crate) fn similarity_label(top: f64) -> String {
    if top.is_finite() {
        format!("{top:.3}")
    } else {
        "none".to_string()
    }
}

/// Ranks `hits` best-first, drops empty content and the weak tail below the
/// relative floor, and keeps at most [`AUTO_RECALL_LIMIT`].
pub(crate) fn select_hits(mut hits: Vec<RetrievalHit>) -> Vec<RetrievalHit> {
    // A NaN or infinite score is a driver bug, not a ranking; it would either
    // sort first (`total_cmp` places NaN above every real score) or defeat the
    // relative floor (`NaN >= floor` is false), so it is dropped outright.
    hits.retain(|hit| hit.score.is_finite() && !hit.content.trim().is_empty());
    // Best first. `total_cmp` is a total order, so a NaN score — which
    // `partial_cmp` cannot place — sorts deterministically (after every real
    // score in descending order) instead of destabilising the sort.
    hits.sort_by(|a, b| b.score.total_cmp(&a.score));
    if let Some(top) = hits.first().map(|hit| hit.score) {
        if top > 0.0 {
            let floor = top * AUTO_RECALL_RELATIVE_FLOOR;
            hits.retain(|hit| hit.score >= floor);
        }
    }
    hits.truncate(AUTO_RECALL_LIMIT);
    hits
}

/// Ranks `notes` by vector similarity, best first, drops the ones below
/// [`AUTO_RECALL_NOTE_MIN_SIMILARITY`], empty bodies and non-finite scores, and
/// keeps at most [`AUTO_RECALL_LIMIT`].
///
/// The floor reads the vector component rather than the engine's combined
/// score for the reason Lane B's `recall_by_vector_over` gives: the combined
/// score folds in keyword, graph and freshness signals, so a lexically similar
/// but semantically unrelated note would otherwise clear the bar.
pub(crate) fn select_notes(mut notes: Vec<NamespaceMemoryHit>) -> Vec<NamespaceMemoryHit> {
    notes.retain(|note| {
        let similarity = note.score_breakdown.vector_similarity;
        similarity.is_finite()
            && similarity >= AUTO_RECALL_NOTE_MIN_SIMILARITY
            && !note.content.trim().is_empty()
    });
    notes.sort_by(|a, b| {
        b.score_breakdown
            .vector_similarity
            .total_cmp(&a.score_breakdown.vector_similarity)
    });
    notes.truncate(AUTO_RECALL_LIMIT);
    notes
}

/// The block the turn prepends: the banner, the hint, one line per note, one
/// line per tree hit, kept within `recall_max_chars` when the guard sets one.
///
/// The budget is spent on whole lines, never on characters: a line that does
/// not fit is left out, so an `<untrusted-source>` marker is never cut in half
/// and every opening marker keeps its closing one. Notes come first — they are
/// what the user asked to keep — so under a tight cap they win over tree hits.
/// The hint is a caption, not the content: it is reserved first, but a cap
/// that then fits no line at all is rendered again without it, so the hint
/// can never be what suppresses the only recalled line. A cap that fits no
/// line even then yields an empty block, since a banner over nothing would be
/// noise.
pub(crate) fn render_block(
    notes: &[NamespaceMemoryHit],
    hits: &[RetrievalHit],
    recall_max_chars: Option<usize>,
) -> String {
    let banner = format!("{AUTO_RECALL_BANNER}\n\n");
    let hint = format!("{AUTO_RECALL_HINT}\n\n");
    let candidates: Vec<String> = notes
        .iter()
        .map(render_note_line)
        .chain(hits.iter().map(render_hit_line))
        .collect();
    // Banner, hint, lines, and the closing blank line all count against the cap.
    let base = banner.chars().count() + 1;
    let hint_cost = hint.chars().count();
    let mut with_hint = fits_within(base, hint_cost, recall_max_chars);
    let mut lines = fit_lines(
        &candidates,
        if with_hint { base + hint_cost } else { base },
        recall_max_chars,
    );
    if with_hint && lines.is_empty() {
        // The hint fit on its own but left no room for a line: a block is its
        // lines, so give the space back and try once more without the caption.
        with_hint = false;
        lines = fit_lines(&candidates, base, recall_max_chars);
    }
    if lines.is_empty() {
        return String::new();
    }
    let mut block = banner;
    if with_hint {
        block.push_str(&hint);
    }
    for line in &lines {
        block.push_str(line);
    }
    block.push('\n');
    block
}

/// Whether `cost` more characters still fit under `recall_max_chars` once
/// `used` are spent. No cap fits everything.
fn fits_within(used: usize, cost: usize, recall_max_chars: Option<usize>) -> bool {
    match recall_max_chars {
        Some(max_chars) => used + cost <= max_chars,
        None => true,
    }
}

/// The `candidates` that fit, in order, once `used` characters are spent:
/// whole lines only, each omitted when it would cross the cap.
fn fit_lines(
    candidates: &[String],
    mut used: usize,
    recall_max_chars: Option<usize>,
) -> Vec<String> {
    let mut lines: Vec<String> = Vec::with_capacity(candidates.len());
    for line in candidates {
        let cost = line.chars().count();
        if !fits_within(used, cost, recall_max_chars) {
            log::debug!(
                "[auto_recall] line omitted: {cost} chars would exceed recall_max_chars={recall_max_chars:?}"
            );
            continue;
        }
        used += cost;
        lines.push(line.clone());
    }
    lines
}

/// One note as a bullet line: the content (one line, capped) and the key it
/// was filed under, so the model can tell a kept note from a passing chunk.
///
/// A note that did not come from the conversation is wrapped whole — content
/// **and** key inside one `<untrusted-source>` marker. The key is model- or
/// provider-supplied text on exactly the same footing as the body (a synced
/// row's key can be an email subject), so a key rendered after the closing
/// marker would be the payload's way back into the trusted region.
fn render_note_line(note: &NamespaceMemoryHit) -> String {
    let mut line = String::from("- ");
    let content = one_line(&note.content, AUTO_RECALL_PER_HIT_CHARS);
    let key = one_line(&note.key, AUTO_RECALL_SCOPE_CHARS);
    let labelled = if key.is_empty() {
        content
    } else {
        format!("{content} (note: {key})")
    };
    match untrusted_note_hint(note) {
        Some(hint) => line.push_str(&wrap_untrusted_for_agent(&labelled, &hint)),
        None => line.push_str(&labelled),
    }
    line.push('\n');
    line
}

/// The source hint to wrap `note` with, or `None` for a note the user or the
/// assistant filed in chat.
///
/// Two signals say a note did not come from the conversation: the
/// `ExternalSync` taint a sync path stamps at write time, and the
/// namespace/key shapes `memory_context_safety` already treats as
/// connector-derived (a `gmail:` key, a namespace off the local-authored
/// allowlist). Either wraps. The hint is the key's prefix when it has one, so
/// the marker names the surface the way source-tree hits do.
fn untrusted_note_hint(note: &NamespaceMemoryHit) -> Option<String> {
    let external = note.taint == MemoryTaint::ExternalSync
        || is_potentially_untrusted(Some(&note.namespace), &note.key);
    if !external {
        return None;
    }
    let key_prefix = note
        .key
        .split_once(':')
        .map(|(prefix, _)| prefix.trim())
        .filter(|prefix| !prefix.is_empty());
    Some(key_prefix.unwrap_or("note").to_string())
}

/// One hit as a bullet line: the content (one line, capped, wrapped when it
/// came from a source tree) and the scope label.
fn render_hit_line(hit: &RetrievalHit) -> String {
    let mut line = String::from("- ");
    let content = one_line(&hit.content, AUTO_RECALL_PER_HIT_CHARS);
    match untrusted_source_hint(hit) {
        Some(hint) => line.push_str(&wrap_untrusted_for_agent(&content, &hint)),
        None => line.push_str(&content),
    }
    // The scope is driver metadata (`folder:profile`, `slack:#eng`), but it
    // lands in the prompt like the content does, so it gets the same one-line
    // treatment and a short cap rather than a trusted pass-through.
    let scope = one_line(&hit.tree_scope, AUTO_RECALL_SCOPE_CHARS);
    if !scope.is_empty() {
        line.push_str(" (from ");
        line.push_str(&scope);
        line.push(')');
    }
    line.push('\n');
    line
}

/// The source hint to wrap `hit` with, or `None` for content the user or the
/// assistant authored in chat.
///
/// A source tree holds ingested content — email, Slack, Notion, a web page, a
/// folder on disk — and an author of any of those can write instructions.
/// Rendered bare, a chunk that says "ignore your earlier instructions" reads
/// with the same authority as the user's own words; the same rule the older
/// recall path applied (`memory_context_safety`) applies here: anything that
/// is not a chat tree is wrapped in the `<untrusted-source>` marker. Default
/// deny — a hit with no tree at all is wrapped too. The hint is the scope's
/// prefix (`gmail`, `slack`, `folder`), sanitised by the wrapper.
fn untrusted_source_hint(hit: &RetrievalHit) -> Option<String> {
    if hit.tree_kind.as_deref() == Some("chat") {
        return None;
    }
    let scope_prefix = hit
        .tree_scope
        .split_once(':')
        .map(|(prefix, _)| prefix.trim())
        .filter(|prefix| !prefix.is_empty());
    Some(
        scope_prefix
            .or(hit.tree_kind.as_deref())
            .unwrap_or("external")
            .to_string(),
    )
}

/// `text` clipped to at most `max_chars` characters **including** `suffix`,
/// which marks the cut. A cap too small to hold the suffix yields a bare
/// prefix of that length; a text that already fits is returned unchanged.
fn clip(text: &str, max_chars: usize, suffix: &str) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let suffix_len = suffix.chars().count();
    if max_chars < suffix_len {
        return text.chars().take(max_chars).collect();
    }
    let mut out: String = text.chars().take(max_chars - suffix_len).collect();
    out.push_str(suffix);
    out
}

/// `text` with its whitespace collapsed onto one line and clipped to
/// `max_chars`, so a multi-paragraph chunk stays one bullet.
fn one_line(text: &str, max_chars: usize) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    clip(&collapsed, max_chars, "…")
}

#[cfg(test)]
#[path = "auto_recall_tests.rs"]
mod tests;
