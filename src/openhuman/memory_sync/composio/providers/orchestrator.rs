//! Generic incremental-sync orchestrator for Composio providers.
//!
//! Every Composio provider (gmail, notion, slack, …) used to hand-roll the
//! same ~15-step sync loop: load [`SyncState`] → check the daily budget →
//! resolve identity/scopes → page through results → dedupe against
//! `synced_ids` → drop items outside the `sync_depth_days` window → clamp to
//! `max_items` → ingest → advance the cursor → persist state → build a
//! [`SyncOutcome`]. The copy-paste is exactly how the page-granular cap bug
//! (#3304) ended up in five providers and was missed in a sixth.
//!
//! [`ItemCap`] (PR #3304) centralised the cap *math*; this module owns it and
//! the *control flow*. Now that every provider rides [`run_sync`], `ItemCap`
//! lives here (orchestrator-private), not in the shared `helpers` module — the
//! cap rule exists in exactly one place. A provider implements only the slim
//! [`IncrementalSource`] primitives and inherits the `max_items` cap, the
//! `sync_depth_days` window, dedup, cursor advance, and budget enforcement for
//! free.
//!
//! ## Scopes: flat AND nested in one loop
//!
//! [`SyncScope`] is the abstraction that lets the same orchestrator drive both
//! shapes:
//!
//!   * **Flat** providers (github, notion, linear) expose a *single implicit
//!     scope* — [`SyncScope::flat`] — and page straight through their one
//!     result stream.
//!   * **Nested** providers (clickup workspaces → tasks, slack channels →
//!     history) resolve their containers in [`IncrementalSource::preamble`] and
//!     return one [`SyncScope`] per container; the orchestrator's
//!     `for scope { for page {…} }` loop is byte-for-byte identical for both.
//!
//! ## Per-scope cursors + error tolerance (Slack)
//!
//! Slack is the structural outlier: it keeps a watermark **per channel** (not a
//! single global cursor) and must not let one bad channel abort the rest. Two
//! opt-in trait hooks — [`IncrementalSource::per_scope_cursors`] and
//! [`IncrementalSource::tolerate_scope_errors`] — bend the same scope loop to
//! that shape without touching the single-cursor providers, which leave both
//! `false` and keep the advance-once-at-the-end path verbatim.

use async_trait::async_trait;
use serde_json::{json, Value};

use super::sync_state::SyncState;
use super::{ProviderContext, SyncOutcome, SyncReason};

/// Compute the number of API pages needed to cover `max_items` at `page_size`
/// items per page, rounding up.
///
/// Returns `u32::MAX` when `page_size == 0` to avoid division by zero;
/// callers should treat this as "no page cap beyond the provider's own upper
/// bound".
fn pages_for_max_items(max_items: u32, page_size: u32) -> u32 {
    if page_size == 0 {
        return u32::MAX;
    }
    // Widen to u64 before the addition to prevent overflow for large cap values.
    (((max_items as u64) + (page_size as u64) - 1) / (page_size as u64)).min(u32::MAX as u64) as u32
}

/// Single source of truth for the per-sync `max_items` cap.
///
/// Every Composio provider used to open-code three near-identical blocks — a
/// page-count cap, a mid-page clamp, and a post-page hard stop — which is how
/// the same off-by-a-page bug (#3304) ended up in five providers and was missed
/// in a sixth. Now that every provider rides [`run_sync`], this type and its
/// page math are **orchestrator-private**: the cap rule lives in exactly one
/// place — construct it from `ctx.max_items`, derive the page cap, clamp each
/// batch before ingest, and stop once the cap is reached.
///
/// `None` cap means "no item limit beyond the provider's own internal page
/// ceiling" (e.g. after the user clicks "All In").
#[derive(Debug, Clone, Copy)]
struct ItemCap {
    cap: Option<usize>,
    persisted: usize,
}

impl ItemCap {
    /// Build from a source's `max_items` value (`None` = uncapped).
    fn new(max_items: Option<u32>) -> Self {
        Self {
            cap: max_items.map(|n| n as usize),
            persisted: 0,
        }
    }

    /// The page ceiling to actually fetch: the smaller of the provider's own
    /// `fallback` (e.g. `MAX_PAGES_PER_SYNC`) and the pages needed to cover the
    /// cap. Uncapped → `fallback` unchanged.
    fn max_pages(&self, page_size: u32, fallback: u32) -> u32 {
        match self.cap {
            Some(cap) => pages_for_max_items(cap as u32, page_size).min(fallback),
            None => fallback,
        }
    }

    /// How many more items may still be persisted. `None` = unlimited.
    fn remaining(&self) -> Option<usize> {
        self.cap.map(|cap| cap.saturating_sub(self.persisted))
    }

    /// True once the cap is set and reached — callers `break` their pagination.
    fn is_reached(&self) -> bool {
        matches!(self.remaining(), Some(0))
    }

    /// Record `n` newly-persisted items against the budget.
    fn record(&mut self, n: usize) {
        self.persisted = self.persisted.saturating_add(n);
    }

    /// Truncate a to-ingest batch down to the remaining budget, so a single
    /// page larger than the cap never over-persists. No-op when uncapped.
    fn clamp_batch<T>(&self, batch: &mut Vec<T>) {
        if let Some(remaining) = self.remaining() {
            if batch.len() > remaining {
                batch.truncate(remaining);
            }
        }
    }
}

/// A unit of work to iterate within one sync pass.
///
/// Flat providers return a single [`SyncScope::flat`]; nested providers return
/// one scope per container (workspace / channel). The orchestrator treats both
/// uniformly — that uniformity is the whole point of the abstraction.
#[derive(Debug, Clone)]
pub(crate) struct SyncScope {
    /// Provider-native scope id (e.g. a ClickUp workspace id or a Slack
    /// channel id). Empty string denotes the single implicit scope of a flat
    /// provider.
    pub id: String,
    /// Human-readable label for logs. Never logged at a level that would leak
    /// PII — ids/labels here are container identifiers, not user content.
    pub label: String,
}

impl SyncScope {
    /// The single implicit scope of a flat provider (gmail/github/notion/linear).
    pub(crate) fn flat() -> Self {
        Self {
            id: String::new(),
            label: "<flat>".to_string(),
        }
    }

    /// One container of a nested provider (clickup workspace, slack channel).
    pub(crate) fn nested(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
        }
    }
}

/// One page fetched from a provider for a given [`SyncScope`].
pub(crate) struct PageFetch {
    /// Raw upstream items, already unwrapped from the Composio envelope by the
    /// provider's [`IncrementalSource::fetch_page`].
    pub items: Vec<Value>,
    /// Opaque next-page cursor for *this scope*, or `None` when this was the
    /// last page. The orchestrator never interprets it — it is a Notion
    /// `start_cursor`, a ClickUp page index, etc., round-tripped back into the
    /// next `fetch_page` call.
    pub next: Option<String>,
}

/// One item that survived dedup + the depth window + the `max_items` clamp and
/// is queued for ingest.
pub(crate) struct SyncItem {
    /// Stable dedup key (e.g. `{page_id}@{edited_time}`). Marked synced on a
    /// successful ingest so the next pass skips it.
    pub dedup_key: String,
    /// Sort timestamp used for cursor advancement and depth-window compares.
    /// Same representation as [`IncrementalSource::depth_floor`].
    pub sort_ts: Option<String>,
    /// Raw upstream payload, handed to [`IncrementalSource::ingest`].
    pub raw: Value,
}

/// Folded result of one [`IncrementalSource::ingest`] call. Every field is
/// order-independent so a concurrent ingest stage can accumulate into it.
#[derive(Default)]
pub(crate) struct IngestOutcome {
    /// Dedup keys whose ingest succeeded — the orchestrator marks each synced.
    pub synced_keys: Vec<String>,
    /// Number of items persisted (equals `synced_keys.len()`).
    pub persisted: usize,
    /// Whether any per-item ingest failed. When true and the source opts into
    /// [`IncrementalSource::hold_cursor_on_ingest_failure`], the orchestrator
    /// holds the cursor so the failed range is re-fetched next pass.
    pub had_failures: bool,
}

/// The slim primitive a Composio provider implements to ride [`run_sync`].
///
/// Implementations own *only* the provider-specific shapes — which actions to
/// call, how to read ids/timestamps, how to persist. The orchestrator owns all
/// the control flow (budget, pagination bound, dedup, depth window, cap, cursor
/// advance, state persistence).
#[async_trait]
pub(crate) trait IncrementalSource: Send + Sync {
    /// Toolkit slug — used for [`SyncState`] keying and log prefixes.
    fn toolkit(&self) -> &'static str;

    /// Page size to request this pass. Providers typically widen this for
    /// [`SyncReason::ConnectionCreated`] to backfill faster.
    fn page_size(&self, reason: SyncReason) -> u32;

    /// The provider's own internal page ceiling — the `fallback` handed to
    /// [`ItemCap::max_pages`], applied *per scope*.
    fn max_pages(&self) -> u32;

    /// The page ceiling for *this pass*, given the reason and current state.
    /// Defaults to the fixed [`Self::max_pages`]; gmail overrides it to apply an
    /// adaptive cap (e.g. only a couple of pages when the previous sync ran very
    /// recently). The orchestrator hands the result to [`ItemCap::max_pages`].
    fn page_ceiling(&self, reason: SyncReason, state: &SyncState) -> u32 {
        let _ = (reason, state);
        self.max_pages()
    }

    /// Whether to stop paginating a scope once a fetched page yields **no new
    /// items** (everything deduped away as already-synced). Default `false` —
    /// the orchestrator keeps paginating until the cursor/depth boundary or the
    /// last page. Gmail overrides to `true`: its result set is ordered so an
    /// all-already-synced page means there is nothing newer left to fetch.
    fn stop_on_empty_pending(&self) -> bool {
        false
    }

    /// Resolve identity and list the scopes to iterate.
    ///
    /// Flat providers return `vec![SyncScope::flat()]`. Nested providers call
    /// their "list workspaces / channels" action(s) here and return one scope
    /// each (recording any budget spent via `state`). Returning an empty vec
    /// short-circuits the pass to a no-op outcome.
    async fn preamble(
        &self,
        ctx: &ProviderContext,
        state: &mut SyncState,
    ) -> Result<Vec<SyncScope>, String>;

    /// Fetch one page of raw items for `scope` at `cursor` (`None` = first
    /// page). Return the items already unwrapped from the Composio envelope
    /// plus the opaque next-page token.
    ///
    /// Implementations **must record the page request against the daily budget**
    /// (`state.record_requests(1)`) for any *completed* round-trip — including
    /// one the provider reports as `successful == false` — before converting it
    /// to an `Err`, so a broken/unauthorized connection cannot make unlimited
    /// billable failed calls without hitting the per-day cap. A transport error
    /// (no round-trip) must not be recorded.
    async fn fetch_page(
        &self,
        ctx: &ProviderContext,
        scope: &SyncScope,
        cursor: Option<&str>,
        reason: SyncReason,
        state: &mut SyncState,
    ) -> Result<PageFetch, String>;

    /// Stable dedup key for one raw item. `None` drops the item (e.g. it has no
    /// extractable id).
    fn item_dedup_key(&self, item: &Value) -> Option<String>;

    /// Sort timestamp for one raw item — compared against the persistent cursor
    /// and the depth floor. `None` means "no timestamp" (never trips the cursor
    /// boundary or the depth window).
    fn item_sort_ts(&self, item: &Value) -> Option<String>;

    /// Build the `sync_depth_days` floor in the *same representation* as
    /// [`Self::item_sort_ts`] so the lexicographic compare is valid. Default is
    /// RFC3339 UTC; providers whose timestamps are epoch-millis strings
    /// (clickup) override. Unused when [`Self::server_side_depth`] is `true`.
    fn depth_floor(&self, days: u32) -> String {
        let floor = chrono::Utc::now() - chrono::Duration::days(days as i64);
        floor.to_rfc3339()
    }

    /// Noun used for this provider's `{noun}_fetched` / `{noun}_persisted`
    /// keys in the [`SyncOutcome::details`] diagnostic blob, preserving each
    /// provider's historical detail shape (notion: `results`, github/linear:
    /// `issues`, clickup: `tasks`, …). `details` is for logging/UI status only;
    /// nothing reads these keys in production.
    fn detail_noun(&self) -> &'static str {
        "results"
    }

    /// Whether the provider applies the `sync_depth_days` window **itself**
    /// (server-side — e.g. GitHub's `updated:>{date}` search qualifier),
    /// rather than relying on the orchestrator's client-side timestamp
    /// truncation. When `true`, the orchestrator skips its client-side depth
    /// filter and the provider must inject the window inside
    /// [`Self::fetch_page`] (typically only on the first sync, before a cursor
    /// exists). Default `false` — the orchestrator filters client-side via
    /// [`Self::depth_floor`].
    fn server_side_depth(&self) -> bool {
        false
    }

    /// Whether to hold (not advance) the cursor when an ingest reported a
    /// failure this pass. Default `true` — Notion's safe behaviour under the
    /// delete-first memory-tree pipeline (#2885), where an edited item that
    /// fails to re-ingest must be re-fetched. Providers that advance regardless
    /// of per-item failures (clickup) override to `false`.
    fn hold_cursor_on_ingest_failure(&self) -> bool {
        true
    }

    /// Whether this source keeps a **cursor per scope** (Slack: one `oldest`
    /// watermark per channel) instead of the single global watermark every
    /// other provider uses. Default `false`.
    ///
    /// When `true` the orchestrator:
    ///   * disables its global-cursor boundary detection in [`select_pending`]
    ///     (the scope's watermark is read by the provider's own
    ///     [`Self::fetch_page`] — usually injected as a server-side `oldest`
    ///     filter, so `server_side_depth` is typically `true` too);
    ///   * tracks the newest timestamp **per scope** and, after each scope
    ///     finishes cleanly, calls [`Self::advance_scope_cursor`] to advance
    ///     that one scope's watermark, persisting state between scopes;
    ///   * holds a scope's watermark (does not advance it) when that scope hit
    ///     an ingest failure or the `max_items` cap truncated it — so the next
    ///     pass re-fetches that scope's unseen/failed tail.
    ///
    /// Single-cursor providers leave this `false` and keep the global
    /// advance-once-at-the-end path verbatim.
    fn per_scope_cursors(&self) -> bool {
        false
    }

    /// Whether a scope-level failure is **non-fatal**. Default `false` — a
    /// `fetch_page` error aborts the whole pass (every single-cursor provider's
    /// behaviour). When `true` (Slack), a `fetch_page` error or a scope's
    /// ingest failure is logged, counted in `scopes_errored`, and the
    /// orchestrator continues to the next scope instead of returning `Err`.
    fn tolerate_scope_errors(&self) -> bool {
        false
    }

    /// Advance the persisted watermark for a single `scope` to `newest_ts`
    /// (per-scope-cursor mode only). The default is a no-op; Slack overrides it
    /// to fold the new watermark into its per-channel cursor map serialized in
    /// [`SyncState::cursor`]. Never called when [`Self::per_scope_cursors`] is
    /// `false`.
    fn advance_scope_cursor(&self, _state: &mut SyncState, _scope: &SyncScope, _newest_ts: &str) {}

    /// Persist a batch of already-filtered items. May spend budget via `state`
    /// (e.g. Notion's per-page body fetch). Returns which dedup keys succeeded
    /// so the orchestrator can mark them synced.
    async fn ingest(
        &self,
        ctx: &ProviderContext,
        scope: &SyncScope,
        state: &mut SyncState,
        items: Vec<SyncItem>,
    ) -> IngestOutcome;
}

/// Current wall-clock time in milliseconds since the UNIX epoch.
fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Build the "nothing to do" outcome used by the budget-exhausted and
/// empty-scopes early returns.
fn skipped_outcome(
    toolkit: &str,
    connection_id: &str,
    reason: SyncReason,
    started_at_ms: u64,
    why: &str,
    details: Value,
) -> SyncOutcome {
    SyncOutcome {
        toolkit: toolkit.to_string(),
        connection_id: Some(connection_id.to_string()),
        reason: reason.as_str().to_string(),
        items_ingested: 0,
        started_at_ms,
        finished_at_ms: now_ms(),
        summary: format!("{toolkit} sync skipped: {why}"),
        details,
    }
}

/// Pure (no I/O) per-page scan: extract dedup key + sort timestamp for each raw
/// item, advance `newest_ts`, detect the persistent-cursor boundary, and drop
/// already-synced items. Returns the survivors plus whether we crossed the
/// cursor boundary (the signal to stop paginating this scope).
///
/// This is the generic form of every provider's old `select_pending`. All
/// order-dependent decisions live here so the (possibly concurrent) ingest
/// stage never has to reason about ordering.
///
/// `boundary_cursor` is the watermark used for cursor-boundary detection.
/// Single-cursor providers pass `state.cursor`; per-scope-cursor providers
/// (Slack) pass `None` to disable the global-cursor boundary entirely — their
/// per-scope watermark is enforced server-side inside `fetch_page` instead.
fn select_pending<S: IncrementalSource + ?Sized>(
    source: &S,
    items: &[Value],
    boundary_cursor: Option<&str>,
    state: &SyncState,
    newest_ts: &mut Option<String>,
) -> (Vec<SyncItem>, bool) {
    let mut hit_cursor_boundary = false;
    let mut pending: Vec<SyncItem> = Vec::new();
    for item in items {
        let Some(dedup_key) = source.item_dedup_key(item) else {
            tracing::debug!(
                toolkit = source.toolkit(),
                "[composio:sync_orch] item missing dedup key, skipping"
            );
            continue;
        };

        let sort_ts = source.item_sort_ts(item);

        // Track the newest timestamp for cursor advancement — for *every* item
        // with a timestamp, including ones we skip as already-synced.
        if let Some(ref ts) = sort_ts {
            if newest_ts.as_ref().is_none_or(|existing| ts > existing) {
                *newest_ts = Some(ts.clone());
            }
        }

        // Older-or-equal to the cursor AND already synced → we have caught up.
        if let (Some(cursor), Some(ts)) = (boundary_cursor, &sort_ts) {
            if ts.as_str() <= cursor && state.is_synced(&dedup_key) {
                hit_cursor_boundary = true;
                continue;
            }
        }

        if state.is_synced(&dedup_key) {
            continue;
        }

        pending.push(SyncItem {
            dedup_key,
            sort_ts,
            raw: item.clone(),
        });
    }
    (pending, hit_cursor_boundary)
}

/// Run one incremental sync end-to-end through the generic orchestrator.
///
/// The provider supplies the [`IncrementalSource`] primitives; everything else
/// — budget, the per-scope page loop bounded by [`ItemCap::max_pages`], dedup,
/// the `sync_depth_days` window, the precise `max_items` clamp, cursor
/// advance/hold, state persistence, and the [`SyncOutcome`] — is owned here.
pub(crate) async fn run_sync<S: IncrementalSource + ?Sized>(
    source: &S,
    ctx: &ProviderContext,
    reason: SyncReason,
) -> Result<SyncOutcome, String> {
    let toolkit = source.toolkit();
    let started_at_ms = now_ms();
    let connection_id = ctx
        .connection_id
        .clone()
        .unwrap_or_else(|| "default".to_string());

    tracing::info!(
        toolkit,
        connection_id = %connection_id,
        reason = reason.as_str(),
        "[composio:sync_orch] incremental sync starting"
    );

    // ── Step 1: load persistent sync state ──────────────────────────────
    let Some(memory) = ctx.memory_client() else {
        return Err(format!("[composio:{toolkit}] memory client not ready"));
    };
    let mut state = SyncState::load(&memory, toolkit, &connection_id).await?;

    // ── Step 2: daily budget pre-check ──────────────────────────────────
    if state.budget_exhausted() {
        tracing::info!(
            toolkit,
            connection_id = %connection_id,
            "[composio:sync_orch] daily request budget exhausted, skipping sync"
        );
        return Ok(skipped_outcome(
            toolkit,
            &connection_id,
            reason,
            started_at_ms,
            "daily budget exhausted",
            json!({ "budget_exhausted": true }),
        ));
    }

    // ── Step 3: preamble — resolve identity + scopes ────────────────────
    let scopes = match source.preamble(ctx, &mut state).await {
        Ok(scopes) => scopes,
        Err(e) => {
            // Persist any budget spent during the preamble before propagating.
            let _ = state.save(&memory).await;
            return Err(e);
        }
    };

    if scopes.is_empty() {
        tracing::info!(
            toolkit,
            connection_id = %connection_id,
            "[composio:sync_orch] no scopes to sync"
        );
        state.save(&memory).await?;
        return Ok(skipped_outcome(
            toolkit,
            &connection_id,
            reason,
            started_at_ms,
            "no scopes to sync",
            json!({ "scopes": 0 }),
        ));
    }

    // ── Step 4: caps + window ───────────────────────────────────────────
    let page_size = source.page_size(reason);
    let mut cap = ItemCap::new(ctx.max_items);
    // `page_ceiling` lets a provider apply a state-aware cap (gmail's adaptive
    // cap); it defaults to the fixed `max_pages`.
    let page_ceiling = source.page_ceiling(reason, &state);
    let effective_max_pages = cap.max_pages(page_size, page_ceiling);
    if ctx.max_items.is_some() && effective_max_pages < page_ceiling {
        tracing::debug!(
            toolkit,
            connection_id = %connection_id,
            max_items = ?ctx.max_items,
            effective_max_pages,
            "[composio:sync_orch] [memory_sync] applying max_items page cap"
        );
    }

    // Server-side-depth providers (GitHub) inject the window into the request
    // in `fetch_page`, so the orchestrator skips its client-side floor for them.
    let depth_floor: Option<String> = if source.server_side_depth() {
        None
    } else {
        ctx.sync_depth_days.map(|days| {
            let floor = source.depth_floor(days);
            tracing::debug!(
                toolkit,
                connection_id = %connection_id,
                sync_depth_days = days,
                oldest_allowed = %floor,
                "[composio:sync_orch] [memory_sync] applying sync_depth_days floor"
            );
            floor
        })
    };

    // ── Step 5: scope × page loop ───────────────────────────────────────
    //
    // `per_scope` providers (Slack) advance a watermark per scope inside the
    // loop and hold it on a cap-truncated / failed scope; single-cursor
    // providers advance one global watermark once, in Step 6. `tolerate`
    // providers continue past a scope-level failure instead of aborting.
    let per_scope = source.per_scope_cursors();
    let tolerate = source.tolerate_scope_errors();

    let mut total_fetched: usize = 0;
    let mut total_persisted: usize = 0;
    let mut newest_ts: Option<String> = None;
    let mut had_ingest_failures = false;
    let mut hit_cap_boundary = false;
    let mut scopes_synced: usize = 0;
    let mut scopes_errored: usize = 0;

    'scopes: for scope in &scopes {
        // The page cursor is per-scope — reset at the top of every scope.
        let mut cursor: Option<String> = None;
        // Newest timestamp observed *within this scope*, for per-scope advance.
        let mut scope_newest_ts: Option<String> = None;
        // This scope hit an ingest failure (hold its watermark, count errored).
        let mut scope_had_failure = false;
        // This scope was truncated by the global cap (hold its watermark).
        let mut scope_hit_cap = false;

        for page_num in 0..effective_max_pages {
            if state.budget_exhausted() {
                tracing::info!(
                    toolkit,
                    scope = %scope.label,
                    page = page_num,
                    "[composio:sync_orch] budget exhausted mid-sync, stopping"
                );
                break 'scopes;
            }

            // `fetch_page` records the page request against the budget (incl.
            // provider-reported failures) per its contract. On error we persist
            // whatever budget/dedup progress we have before propagating —
            // parity with the per-provider loops, which saved state before
            // returning a failed-page error. When the source tolerates
            // scope-level errors (Slack), we instead log, count, persist, and
            // move on to the next scope.
            let fetch = match source
                .fetch_page(ctx, scope, cursor.as_deref(), reason, &mut state)
                .await
            {
                Ok(fetch) => fetch,
                Err(e) => {
                    if tolerate {
                        tracing::warn!(
                            toolkit,
                            connection_id = %connection_id,
                            scope = %scope.label,
                            page = page_num,
                            error = %e,
                            "[composio:sync_orch] scope fetch failed (continuing with next scope)"
                        );
                        scopes_errored += 1;
                        let _ = state.save(&memory).await;
                        continue 'scopes;
                    }
                    let _ = state.save(&memory).await;
                    return Err(e);
                }
            };
            total_fetched += fetch.items.len();

            if fetch.items.is_empty() {
                tracing::debug!(
                    toolkit,
                    scope = %scope.label,
                    page = page_num,
                    "[composio:sync_orch] empty page, moving on"
                );
                break;
            }

            // Dedup + cursor-boundary detection + newest-ts tracking. Per-scope
            // providers disable the global-cursor boundary (pass `None`) and
            // accumulate the newest ts into their per-scope tracker.
            let boundary_cursor = if per_scope {
                None
            } else {
                state.cursor.as_deref()
            };
            let ts_acc = if per_scope {
                &mut scope_newest_ts
            } else {
                &mut newest_ts
            };
            let (mut pending, mut hit_cursor_boundary) =
                select_pending(source, &fetch.items, boundary_cursor, &state, ts_acc);

            // A non-empty page whose items all deduped away as already-synced
            // (the gmail "all already synced" signal — captured before the
            // depth/cap filters narrow `pending` for other reasons).
            let page_had_no_new_items = pending.is_empty();

            // sync_depth_days: `pending` is in descending-timestamp order, so
            // truncate at the first item below the floor and stop paginating.
            if let Some(ref floor) = depth_floor {
                if let Some(cut) = pending.iter().position(|it| {
                    it.sort_ts
                        .as_deref()
                        .map(|t| t < floor.as_str())
                        .unwrap_or(false)
                }) {
                    pending.truncate(cut);
                    hit_cursor_boundary = true;
                }
            }

            // max_items: clamp the dedup'd batch to the remaining budget BEFORE
            // ingest — the precise cap that fixes the page-granular #3304 bug.
            cap.clamp_batch(&mut pending);

            // Provider-specific persistence (may spend budget, e.g. body fetch).
            let outcome = source.ingest(ctx, scope, &mut state, pending).await;
            for key in &outcome.synced_keys {
                state.mark_synced(key);
            }
            total_persisted += outcome.persisted;
            cap.record(outcome.persisted);
            if outcome.had_failures {
                had_ingest_failures = true;
                scope_had_failure = true;
            }

            // Precise cap reached → stop the entire pass (after settling this
            // scope's watermark below).
            if cap.is_reached() {
                hit_cap_boundary = true;
                scope_hit_cap = true;
                break;
            }

            if hit_cursor_boundary {
                tracing::debug!(
                    toolkit,
                    scope = %scope.label,
                    page = page_num,
                    "[composio:sync_orch] reached cursor/depth boundary, stopping scope"
                );
                break;
            }

            if page_had_no_new_items && source.stop_on_empty_pending() {
                tracing::debug!(
                    toolkit,
                    scope = %scope.label,
                    page = page_num,
                    "[composio:sync_orch] page had no new items, stopping scope"
                );
                break;
            }

            cursor = fetch.next;
            if cursor.is_none() {
                tracing::debug!(
                    toolkit,
                    scope = %scope.label,
                    page = page_num,
                    "[composio:sync_orch] no next cursor, scope done"
                );
                break;
            }
        }

        // ── Per-scope watermark settle (per_scope providers only) ────────
        if per_scope {
            if scope_had_failure {
                // Ingest failure → hold this scope's watermark, count errored.
                scopes_errored += 1;
                tracing::warn!(
                    toolkit,
                    connection_id = %connection_id,
                    scope = %scope.label,
                    "[composio:sync_orch] scope ingest failed; watermark held, re-fetch next pass"
                );
            } else {
                if scope_hit_cap {
                    // Cap truncated this scope → hold its watermark so the next
                    // pass re-scans the unseen tail; still a processed scope.
                    tracing::debug!(
                        toolkit,
                        scope = %scope.label,
                        "[composio:sync_orch] cap truncated scope; watermark held"
                    );
                } else if let Some(ref nt) = scope_newest_ts {
                    source.advance_scope_cursor(&mut state, scope, nt);
                }
                scopes_synced += 1;
            }
            // Persist between scopes for crash-resilience (parity with the
            // per-provider Slack loop, which saved state after each channel).
            if let Err(err) = state.save(&memory).await {
                tracing::warn!(
                    toolkit,
                    error = %err,
                    "[composio:sync_orch] per-scope state save failed (non-fatal)"
                );
            }
        }

        if scope_hit_cap {
            break 'scopes;
        }
    }

    // ── Step 6: advance cursor (or hold) and persist state ──────────────
    //
    // Per-scope providers already advanced/held each scope's watermark inline,
    // so the global advance is skipped for them. Single-cursor providers hold
    // the global cursor on a cap-truncated pass so the next sync re-scans the
    // unseen tail, and on an ingest failure when the source opts in (Notion's
    // delete-first safety). Otherwise advance to the newest timestamp seen.
    if !per_scope {
        let hold_cursor =
            hit_cap_boundary || (had_ingest_failures && source.hold_cursor_on_ingest_failure());
        if !hold_cursor {
            if let Some(new_cursor) = newest_ts {
                state.advance_cursor(&new_cursor);
            }
        } else {
            tracing::warn!(
                toolkit,
                connection_id = %connection_id,
                had_ingest_failures,
                hit_cap_boundary,
                "[composio:sync_orch] holding cursor — cap-truncated pass or ingest failures; \
                 next sync will re-fetch the unseen/failed range"
            );
        }
    }
    state.set_last_sync_at_ms(now_ms());
    state.save(&memory).await?;

    let finished_at_ms = now_ms();
    let summary = format!(
        "{toolkit} sync ({reason}): fetched {total_fetched}, persisted {total_persisted} new, \
         budget remaining {remaining}",
        reason = reason.as_str(),
        remaining = state.budget_remaining(),
    );
    tracing::info!(
        toolkit,
        connection_id = %connection_id,
        elapsed_ms = finished_at_ms.saturating_sub(started_at_ms),
        total_fetched,
        total_persisted,
        budget_remaining = state.budget_remaining(),
        "[composio:sync_orch] incremental sync complete"
    );

    // Provider-named `{noun}_fetched` / `{noun}_persisted` keys preserve each
    // provider's historical `details` shape (notion `results`, github/linear
    // `issues`, …). Built dynamically since `json!` can't take runtime keys.
    let noun = source.detail_noun();
    let mut details = json!({
        "budget_remaining": state.budget_remaining(),
        "cursor": state.cursor,
        "synced_ids_total": state.synced_ids.len(),
    });
    if let Some(obj) = details.as_object_mut() {
        obj.insert(format!("{noun}_fetched"), json!(total_fetched));
        obj.insert(format!("{noun}_persisted"), json!(total_persisted));
        // Scope accounting is only meaningful for providers that iterate real
        // per-scope work with tolerance/advance (Slack); emitting it for
        // single-cursor providers would change their historical `details` shape.
        if per_scope || tolerate {
            obj.insert("scopes_total".to_string(), json!(scopes.len()));
            obj.insert("scopes_synced".to_string(), json!(scopes_synced));
            obj.insert("scopes_errored".to_string(), json!(scopes_errored));
        }
    }

    Ok(SyncOutcome {
        toolkit: toolkit.to_string(),
        connection_id: Some(connection_id),
        reason: reason.as_str().to_string(),
        items_ingested: total_persisted,
        started_at_ms,
        finished_at_ms,
        summary,
        details,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::config::Config;
    use std::sync::Arc;
    use tempfile::TempDir;

    /// A minimal in-test [`IncrementalSource`] that proves a *future* toolkit
    /// inherits the cap + window for free. It synthesises items per scope (or
    /// returns explicit ones) and "ingests" by counting — no real memory tree.
    /// `Default` keeps the per-test literals to just the field(s) they vary.
    #[derive(Default)]
    struct FakeSource {
        scopes: Vec<SyncScope>,
        items_per_scope: usize,
        /// When set, returned verbatim for the first page of the (single) scope
        /// instead of synthesised items — used by the depth-window test.
        explicit_items: Option<Vec<Value>>,
        /// When true, `preamble` returns an error (exercises the preamble-error
        /// save-and-propagate path).
        fail_preamble: bool,
        /// When true, `fetch_page` returns a *transport* error (no round-trip →
        /// not budget-recorded).
        fail_fetch: bool,
        /// When true, `fetch_page` records the round-trip *then* returns a
        /// provider-reported failure — pins that a failed page still consumes
        /// the daily budget.
        provider_fail_fetch: bool,
        /// When true, advertise server-side depth so the orchestrator skips its
        /// client-side window filter (GitHub's behaviour).
        server_side_depth: bool,
        /// When true, keep a per-scope watermark map in `state.cursor`
        /// (Slack's behaviour) instead of a single global cursor.
        per_scope: bool,
        /// When true, a scope-level fetch failure is non-fatal — the pass
        /// continues to the next scope and records `scopes_errored`.
        tolerate: bool,
        /// Scope id whose `fetch_page` returns a transport error (used to
        /// exercise per-scope error tolerance). `None` → every scope succeeds.
        fail_scope: Option<String>,
        /// When `Some`, drive multi-page returns: page `i` returns `pages[i]`
        /// with the opaque cursor = next page index. Overrides the single-page
        /// synth path.
        pages: Option<Vec<Vec<Value>>>,
        /// When true, override `stop_on_empty_pending()` (gmail's behaviour).
        stop_on_empty: bool,
        /// When `Some`, override `page_ceiling()` (gmail's adaptive cap).
        page_ceiling: Option<u32>,
    }

    impl FakeSource {
        fn flat(items_per_scope: usize) -> Self {
            Self {
                scopes: vec![SyncScope::flat()],
                items_per_scope,
                ..Default::default()
            }
        }
    }

    #[async_trait]
    impl IncrementalSource for FakeSource {
        fn toolkit(&self) -> &'static str {
            "faketoolkit"
        }
        fn page_size(&self, _reason: SyncReason) -> u32 {
            50
        }
        fn max_pages(&self) -> u32 {
            20
        }
        fn stop_on_empty_pending(&self) -> bool {
            self.stop_on_empty
        }
        fn page_ceiling(&self, _reason: SyncReason, _state: &SyncState) -> u32 {
            self.page_ceiling.unwrap_or_else(|| self.max_pages())
        }
        async fn preamble(
            &self,
            _ctx: &ProviderContext,
            state: &mut SyncState,
        ) -> Result<Vec<SyncScope>, String> {
            if self.fail_preamble {
                // Spend a request first so the save-on-error path persists it.
                state.record_requests(1);
                return Err("fake preamble failure".to_string());
            }
            Ok(self.scopes.clone())
        }
        fn per_scope_cursors(&self) -> bool {
            self.per_scope
        }
        fn tolerate_scope_errors(&self) -> bool {
            self.tolerate
        }
        fn advance_scope_cursor(&self, state: &mut SyncState, scope: &SyncScope, newest_ts: &str) {
            // Mirror Slack: fold the watermark into a per-scope map serialized
            // in `state.cursor`.
            let mut map: std::collections::BTreeMap<String, String> = state
                .cursor
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or_default();
            map.insert(scope.id.clone(), newest_ts.to_string());
            state.cursor = Some(serde_json::to_string(&map).unwrap());
        }
        async fn fetch_page(
            &self,
            _ctx: &ProviderContext,
            scope: &SyncScope,
            cursor: Option<&str>,
            _reason: SyncReason,
            state: &mut SyncState,
        ) -> Result<PageFetch, String> {
            if self.fail_scope.as_deref() == Some(scope.id.as_str()) {
                // A scope-level transport error — exercises tolerate_scope_errors.
                return Err(format!("fake scope fetch failure for {}", scope.id));
            }
            if self.fail_fetch {
                // Simulate a transport error (no completed round-trip) → not
                // recorded, matching the contract.
                return Err("fake fetch_page failure".to_string());
            }
            // A completed round-trip — record it against the budget.
            state.record_requests(1);
            if self.provider_fail_fetch {
                // Completed but the provider reported failure — already recorded.
                return Err("fake provider-reported page failure".to_string());
            }
            // Multi-page mode: cursor is the page index.
            if let Some(pages) = &self.pages {
                let idx: usize = cursor.and_then(|c| c.parse().ok()).unwrap_or(0);
                let items = pages.get(idx).cloned().unwrap_or_default();
                let next = (idx + 1 < pages.len()).then(|| (idx + 1).to_string());
                return Ok(PageFetch { items, next });
            }
            // Single page per scope: everything comes back on the first call.
            if cursor.is_some() {
                return Ok(PageFetch {
                    items: vec![],
                    next: None,
                });
            }
            if let Some(items) = &self.explicit_items {
                return Ok(PageFetch {
                    items: items.clone(),
                    next: None,
                });
            }
            let items = (0..self.items_per_scope)
                .map(|i| {
                    json!({
                        "id": format!("{}-{i}", scope.id),
                        "ts": "2099-01-01T00:00:00Z"
                    })
                })
                .collect();
            Ok(PageFetch { items, next: None })
        }
        fn item_dedup_key(&self, item: &Value) -> Option<String> {
            item.get("id").and_then(Value::as_str).map(str::to_string)
        }
        fn item_sort_ts(&self, item: &Value) -> Option<String> {
            item.get("ts").and_then(Value::as_str).map(str::to_string)
        }
        fn server_side_depth(&self) -> bool {
            self.server_side_depth
        }
        async fn ingest(
            &self,
            _ctx: &ProviderContext,
            _scope: &SyncScope,
            _state: &mut SyncState,
            items: Vec<SyncItem>,
        ) -> IngestOutcome {
            let synced_keys: Vec<String> = items.into_iter().map(|it| it.dedup_key).collect();
            let persisted = synced_keys.len();
            IngestOutcome {
                synced_keys,
                persisted,
                had_failures: false,
            }
        }
    }

    fn fake_ctx(
        tmp: &TempDir,
        max_items: Option<u32>,
        sync_depth_days: Option<u32>,
    ) -> ProviderContext {
        let mut config = Config {
            config_path: tmp.path().join("config.toml"),
            workspace_dir: tmp.path().join("workspace"),
            ..Config::default()
        };
        config.secrets.encrypt = false;
        ProviderContext {
            config: Arc::new(config),
            toolkit: "faketoolkit".to_string(),
            connection_id: Some("conn-fake".to_string()),
            usage: Default::default(),
            max_items,
            sync_depth_days,
        }
    }

    #[tokio::test]
    async fn max_items_caps_ingest_to_exact_count_not_page_granular() {
        let tmp = TempDir::new().unwrap();
        let ctx = fake_ctx(&tmp, Some(2), None);
        // One page returns 5 items; the cap is 2.
        let outcome = run_sync(&FakeSource::flat(5), &ctx, SyncReason::ConnectionCreated)
            .await
            .expect("run_sync");
        assert_eq!(
            outcome.items_ingested, 2,
            "max_items=2 must clamp a 5-item page to EXACTLY 2 (the #3304 fix)"
        );
    }

    #[tokio::test]
    async fn no_cap_ingests_the_full_page() {
        let tmp = TempDir::new().unwrap();
        let ctx = fake_ctx(&tmp, None, None);
        let outcome = run_sync(&FakeSource::flat(5), &ctx, SyncReason::Periodic)
            .await
            .expect("run_sync");
        assert_eq!(
            outcome.items_ingested, 5,
            "with no cap every valid page item is ingested"
        );
    }

    #[tokio::test]
    async fn sync_depth_days_filters_items_below_the_floor() {
        let tmp = TempDir::new().unwrap();
        let ctx = fake_ctx(&tmp, None, Some(7));
        // Descending timestamp order: two recent (far future), three ancient.
        // With a 7-day floor only the two recent items survive.
        let items = vec![
            json!({ "id": "a", "ts": "2099-01-02T00:00:00Z" }),
            json!({ "id": "b", "ts": "2099-01-01T00:00:00Z" }),
            json!({ "id": "c", "ts": "2000-01-03T00:00:00Z" }),
            json!({ "id": "d", "ts": "2000-01-02T00:00:00Z" }),
            json!({ "id": "e", "ts": "2000-01-01T00:00:00Z" }),
        ];
        let source = FakeSource {
            scopes: vec![SyncScope::flat()],
            explicit_items: Some(items),
            ..Default::default()
        };
        let outcome = run_sync(&source, &ctx, SyncReason::Manual)
            .await
            .expect("run_sync");
        assert_eq!(
            outcome.items_ingested, 2,
            "sync_depth_days=7 must drop the three ancient items"
        );
    }

    #[tokio::test]
    async fn server_side_depth_skips_the_client_side_filter() {
        // Same ancient items, but the source advertises server-side depth — so
        // the orchestrator must NOT client-side-truncate (the provider would
        // have filtered in fetch_page). All five survive here.
        let tmp = TempDir::new().unwrap();
        let ctx = fake_ctx(&tmp, None, Some(7));
        let items = vec![
            json!({ "id": "a", "ts": "2099-01-02T00:00:00Z" }),
            json!({ "id": "b", "ts": "2000-01-01T00:00:00Z" }),
            json!({ "id": "c", "ts": "2000-01-02T00:00:00Z" }),
        ];
        let source = FakeSource {
            scopes: vec![SyncScope::flat()],
            explicit_items: Some(items),
            server_side_depth: true,
            ..Default::default()
        };
        let outcome = run_sync(&source, &ctx, SyncReason::Manual)
            .await
            .expect("run_sync");
        assert_eq!(
            outcome.items_ingested, 3,
            "server_side_depth must skip the orchestrator's client-side window filter"
        );
    }

    /// Two pages of items, keyed `id`+`ts`.
    fn page(prefix: &str, n: usize) -> Vec<Value> {
        (0..n)
            .map(|i| json!({ "id": format!("{prefix}{i}"), "ts": "2099-01-01T00:00:00Z" }))
            .collect()
    }

    #[tokio::test]
    async fn page_ceiling_caps_the_pages_fetched() {
        let tmp = TempDir::new().unwrap();
        let ctx = fake_ctx(&tmp, None, None);
        // Two pages of 2 items each. With a page ceiling of 1 (gmail's adaptive
        // cap), only the first page is fetched → 2 ingested, not 4.
        let source = FakeSource {
            scopes: vec![SyncScope::flat()],
            pages: Some(vec![page("a", 2), page("b", 2)]),
            page_ceiling: Some(1),
            ..Default::default()
        };
        let outcome = run_sync(&source, &ctx, SyncReason::Periodic)
            .await
            .expect("run_sync");
        assert_eq!(
            outcome.items_ingested, 2,
            "page_ceiling=1 must fetch only the first page"
        );
    }

    #[tokio::test]
    async fn stop_on_empty_pending_halts_at_an_all_synced_page() {
        let tmp = TempDir::new().unwrap();
        // Page 0 = [a0,a1] (new), page 1 = same ids (all synced after page 0),
        // page 2 = [c0,c1] (new). With stop_on_empty the pass halts at page 1
        // and never reaches page 2 → 2 ingested; without it, page 2's items are
        // also ingested → 4.
        let pages = vec![page("a", 2), page("a", 2), page("c", 2)];

        let stop = FakeSource {
            scopes: vec![SyncScope::flat()],
            pages: Some(pages.clone()),
            stop_on_empty: true,
            ..Default::default()
        };
        let stop_ctx = fake_ctx(&tmp, None, None);
        let stopped = run_sync(&stop, &stop_ctx, SyncReason::Periodic)
            .await
            .expect("run_sync");
        assert_eq!(
            stopped.items_ingested, 2,
            "stop_on_empty must halt at the all-synced page before page 2"
        );

        // Control: same pages, no stop_on_empty → keeps going to page 2.
        let tmp2 = TempDir::new().unwrap();
        let keep = FakeSource {
            scopes: vec![SyncScope::flat()],
            pages: Some(pages),
            ..Default::default()
        };
        let keep_ctx = fake_ctx(&tmp2, None, None);
        let kept = run_sync(&keep, &keep_ctx, SyncReason::Periodic)
            .await
            .expect("run_sync");
        assert_eq!(
            kept.items_ingested, 4,
            "without stop_on_empty the pass continues past the all-synced page"
        );
    }

    #[tokio::test]
    async fn nested_scopes_share_one_cap_budget() {
        let tmp = TempDir::new().unwrap();
        let ctx = fake_ctx(&tmp, Some(4), None);
        // Two scopes, 3 items each (6 total); the cap is 4 → 3 from scope one,
        // 1 from scope two, then the pass stops. Proves the cap spans scopes.
        let source = FakeSource {
            scopes: vec![
                SyncScope::nested("s1", "Scope 1"),
                SyncScope::nested("s2", "Scope 2"),
            ],
            items_per_scope: 3,
            ..Default::default()
        };
        let outcome = run_sync(&source, &ctx, SyncReason::ConnectionCreated)
            .await
            .expect("run_sync");
        assert_eq!(
            outcome.items_ingested, 4,
            "max_items must cap the combined ingest across nested scopes"
        );
    }

    #[tokio::test]
    async fn budget_exhausted_short_circuits_to_a_skip_outcome() {
        let tmp = TempDir::new().unwrap();
        let ctx = fake_ctx(&tmp, None, None);
        // Drain the daily budget before the run so the pre-check trips.
        {
            let memory = ctx.memory_client().expect("memory client");
            let mut state = SyncState::load(&memory, "faketoolkit", "conn-fake")
                .await
                .unwrap();
            state.record_requests(state.budget_remaining());
            state.save(&memory).await.unwrap();
        }
        let outcome = run_sync(&FakeSource::flat(5), &ctx, SyncReason::Periodic)
            .await
            .expect("run_sync");
        assert_eq!(outcome.items_ingested, 0);
        assert!(
            outcome.summary.contains("budget"),
            "exhausted-budget run must report a skip, got: {}",
            outcome.summary
        );
    }

    #[tokio::test]
    async fn empty_scopes_short_circuit_to_a_skip_outcome() {
        let tmp = TempDir::new().unwrap();
        let ctx = fake_ctx(&tmp, None, None);
        let source = FakeSource {
            scopes: vec![], // preamble resolved no scopes to iterate
            items_per_scope: 5,
            ..Default::default()
        };
        let outcome = run_sync(&source, &ctx, SyncReason::Periodic)
            .await
            .expect("run_sync");
        assert_eq!(outcome.items_ingested, 0);
        assert!(
            outcome.summary.contains("no scopes"),
            "empty scopes must report a skip, got: {}",
            outcome.summary
        );
    }

    #[tokio::test]
    async fn preamble_error_propagates() {
        let tmp = TempDir::new().unwrap();
        let ctx = fake_ctx(&tmp, None, None);
        let source = FakeSource {
            scopes: vec![SyncScope::flat()],
            items_per_scope: 5,
            fail_preamble: true,
            ..Default::default()
        };
        let err = run_sync(&source, &ctx, SyncReason::Periodic)
            .await
            .expect_err("preamble failure must propagate");
        assert!(err.contains("preamble"), "got: {err}");
    }

    #[tokio::test]
    async fn fetch_page_error_propagates() {
        let tmp = TempDir::new().unwrap();
        let ctx = fake_ctx(&tmp, None, None);
        let source = FakeSource {
            scopes: vec![SyncScope::flat()],
            items_per_scope: 5,
            fail_fetch: true,
            ..Default::default()
        };
        let err = run_sync(&source, &ctx, SyncReason::Periodic)
            .await
            .expect_err("fetch_page failure must propagate");
        assert!(err.contains("fetch_page"), "got: {err}");
    }

    #[tokio::test]
    async fn provider_reported_page_failure_still_consumes_budget() {
        // Parity with the per-provider loops (and the Codex review): a page that
        // completes the round-trip but reports `successful == false` must count
        // against the daily budget before the error propagates, so a broken
        // connection can't make unlimited billable failed page calls.
        let tmp = TempDir::new().unwrap();
        let ctx = fake_ctx(&tmp, None, None);
        let source = FakeSource {
            scopes: vec![SyncScope::flat()],
            items_per_scope: 5,
            provider_fail_fetch: true,
            ..Default::default()
        };
        let before = {
            let memory = ctx.memory_client().expect("memory client");
            SyncState::load(&memory, "faketoolkit", "conn-fake")
                .await
                .unwrap()
                .budget_remaining()
        };
        let err = run_sync(&source, &ctx, SyncReason::Periodic)
            .await
            .expect_err("provider-reported failure must propagate");
        assert!(err.contains("provider-reported"), "got: {err}");
        // The orchestrator saved state on the page error; the failed page was
        // recorded, so exactly one request was consumed.
        let memory = ctx.memory_client().expect("memory client");
        let after = SyncState::load(&memory, "faketoolkit", "conn-fake")
            .await
            .unwrap()
            .budget_remaining();
        assert_eq!(
            before - after,
            1,
            "a completed-but-failed page must consume exactly one budget request"
        );
    }

    #[test]
    fn select_pending_tracks_newest_skips_synced_and_detects_boundary() {
        let source = FakeSource::flat(0);
        let mut state = SyncState::new("faketoolkit", "conn1");
        state.cursor = Some("2026-04-15T00:00:00Z".to_string());
        // Item B is already synced and older than the cursor.
        state.mark_synced("b");

        let items = vec![
            json!({ "id": "a", "ts": "2026-05-01T00:00:00Z" }),
            json!({ "id": "b", "ts": "2026-04-01T00:00:00Z" }),
            json!({ "ts": "2026-03-01T00:00:00Z" }), // no id → dropped
        ];

        let mut newest: Option<String> = None;
        let (pending, hit_boundary) = select_pending(
            &source,
            &items,
            state.cursor.as_deref(),
            &state,
            &mut newest,
        );

        assert_eq!(pending.len(), 1, "only the new item A survives");
        assert_eq!(pending[0].dedup_key, "a");
        assert!(
            hit_boundary,
            "older synced item B trips the cursor boundary"
        );
        assert_eq!(newest.as_deref(), Some("2026-05-01T00:00:00Z"));
    }

    #[tokio::test]
    async fn per_scope_cursors_advance_each_scope_independently() {
        let tmp = TempDir::new().unwrap();
        let ctx = fake_ctx(&tmp, None, None);
        let source = FakeSource {
            scopes: vec![
                SyncScope::nested("s1", "Scope 1"),
                SyncScope::nested("s2", "Scope 2"),
            ],
            items_per_scope: 3,
            per_scope: true,
            ..Default::default()
        };
        let outcome = run_sync(&source, &ctx, SyncReason::Periodic)
            .await
            .expect("run_sync");
        assert_eq!(outcome.items_ingested, 6, "both scopes' items ingested");
        assert_eq!(outcome.details["scopes_synced"], 2);
        assert_eq!(outcome.details["scopes_errored"], 0);

        // The persisted cursor is a per-scope watermark map carrying *both*
        // scopes — proof the orchestrator advanced each scope independently.
        let memory = ctx.memory_client().expect("memory client");
        let state = SyncState::load(&memory, "faketoolkit", "conn-fake")
            .await
            .unwrap();
        let map: std::collections::BTreeMap<String, String> =
            serde_json::from_str(state.cursor.as_deref().unwrap()).expect("cursor map");
        assert_eq!(map.len(), 2, "both scopes have a watermark");
        assert!(map.contains_key("s1") && map.contains_key("s2"));
    }

    #[tokio::test]
    async fn tolerate_scope_errors_continues_past_a_failed_scope() {
        let tmp = TempDir::new().unwrap();
        let ctx = fake_ctx(&tmp, None, None);
        // Scope s1 fails its fetch; s2 succeeds. With tolerance the pass
        // ingests s2's items, counts s1 errored, and never returns Err.
        let source = FakeSource {
            scopes: vec![
                SyncScope::nested("s1", "Scope 1"),
                SyncScope::nested("s2", "Scope 2"),
            ],
            items_per_scope: 3,
            per_scope: true,
            tolerate: true,
            fail_scope: Some("s1".to_string()),
            ..Default::default()
        };
        let outcome = run_sync(&source, &ctx, SyncReason::Periodic)
            .await
            .expect("tolerant run_sync must not error");
        assert_eq!(
            outcome.items_ingested, 3,
            "only the healthy scope's items are ingested"
        );
        assert_eq!(outcome.details["scopes_errored"], 1);
        assert_eq!(outcome.details["scopes_synced"], 1);

        // The failed scope advanced no watermark; only the healthy one did.
        let memory = ctx.memory_client().expect("memory client");
        let state = SyncState::load(&memory, "faketoolkit", "conn-fake")
            .await
            .unwrap();
        let map: std::collections::BTreeMap<String, String> =
            serde_json::from_str(state.cursor.as_deref().unwrap_or("{}")).expect("cursor map");
        assert!(map.contains_key("s2"), "healthy scope advanced");
        assert!(!map.contains_key("s1"), "failed scope watermark held");
    }

    #[tokio::test]
    async fn per_scope_fatal_when_tolerance_off() {
        // Same failing scope, but tolerance off → the fetch error aborts the
        // whole pass (single-cursor providers' default behaviour).
        let tmp = TempDir::new().unwrap();
        let ctx = fake_ctx(&tmp, None, None);
        let source = FakeSource {
            scopes: vec![SyncScope::nested("s1", "Scope 1")],
            items_per_scope: 3,
            per_scope: true,
            tolerate: false,
            fail_scope: Some("s1".to_string()),
            ..Default::default()
        };
        let err = run_sync(&source, &ctx, SyncReason::Periodic)
            .await
            .expect_err("without tolerance a scope fetch error propagates");
        assert!(err.contains("scope fetch failure"), "got: {err}");
    }

    // ── ItemCap / pages_for_max_items (relocated from helpers.rs, #3902) ──

    #[test]
    fn pages_for_max_items_rounds_up() {
        assert_eq!(pages_for_max_items(100, 25), 4);
        assert_eq!(pages_for_max_items(101, 25), 5);
        assert_eq!(pages_for_max_items(1, 25), 1);
        assert_eq!(pages_for_max_items(50, 50), 1);
        assert_eq!(pages_for_max_items(51, 50), 2);
    }

    #[test]
    fn pages_for_max_items_zero_page_size() {
        assert_eq!(pages_for_max_items(100, 0), u32::MAX);
    }

    #[test]
    fn item_cap_uncapped_is_never_reached() {
        let mut cap = ItemCap::new(None);
        assert_eq!(cap.remaining(), None);
        assert!(!cap.is_reached());
        cap.record(1_000_000);
        assert!(!cap.is_reached());
        assert_eq!(
            cap.max_pages(25, 20),
            20,
            "uncapped keeps the provider fallback"
        );
    }

    #[test]
    fn item_cap_tracks_remaining_and_reached() {
        let mut cap = ItemCap::new(Some(3));
        assert_eq!(cap.remaining(), Some(3));
        assert!(!cap.is_reached());
        cap.record(2);
        assert_eq!(cap.remaining(), Some(1));
        assert!(!cap.is_reached());
        cap.record(5); // saturates, never underflows
        assert_eq!(cap.remaining(), Some(0));
        assert!(cap.is_reached());
    }

    #[test]
    fn item_cap_max_pages_is_min_of_fallback_and_needed() {
        // cap=2, page_size=50 → 1 page needed, well under the fallback.
        assert_eq!(ItemCap::new(Some(2)).max_pages(50, 20), 1);
        // cap=1000, page_size=25 → 40 pages needed, clamped to fallback 20.
        assert_eq!(ItemCap::new(Some(1000)).max_pages(25, 20), 20);
    }

    #[test]
    fn item_cap_clamp_batch_truncates_to_remaining() {
        let cap = ItemCap::new(Some(2));
        let mut batch = vec![1, 2, 3, 4, 5];
        cap.clamp_batch(&mut batch);
        assert_eq!(batch, vec![1, 2]);

        // Uncapped leaves the batch untouched.
        let mut full = vec![1, 2, 3];
        ItemCap::new(None).clamp_batch(&mut full);
        assert_eq!(full, vec![1, 2, 3]);

        // After recording progress, clamp uses the reduced budget.
        let mut cap2 = ItemCap::new(Some(5));
        cap2.record(3);
        let mut batch2 = vec![1, 2, 3, 4];
        cap2.clamp_batch(&mut batch2);
        assert_eq!(batch2, vec![1, 2], "only 2 of the 5 budget remained");
    }
}
