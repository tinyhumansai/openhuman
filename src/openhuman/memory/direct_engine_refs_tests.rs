//! Enforcement lint: the set of production files that call `tinymemory-core`
//! **directly**, around the module seam, must not grow.
//!
//! # Why a second path to memory is a correctness problem, not a size problem
//!
//! `memory::binding` says it plainly: "the built-in driver is the compiled
//! TinyMemory TinyBus module. The host no longer exposes an embedded engine
//! class for memory." Every call that goes over that bus is round-tripped
//! through [`crate::openhuman::memory::api::wire`]'s error table — the one
//! `modules/memory.rs` keeps shared "because reimplementing the mapping here is
//! what would let a `PathEscape` arrive as an `Invalid`, silently reclassifying
//! a sandbox escape as a caller mistake" — and is filtered by the capability
//! set `ModuleMemoryProvider::verify` cross-checks against the module's own
//! answer.
//!
//! A direct `tinymemory_core::…` call gets neither. It is a second, unpoliced
//! door into the same subsystem, and two doors into one capability is a
//! capability whose behaviour can diverge. That is the disease, and it is the
//! only reason this lint still exists.
//!
//! **The symptom is gone: `tinymemory-core` is no longer linked into the
//! shipped binary** (#5560). It used to cost 1.44 MB of `.text` as the 7th
//! largest crate, and that sentence stood here for as long as any entry
//! remained. It no longer follows. The crate left `[dependencies]` on
//! 2026-08-31 while this list still held nine entries, because **every
//! surviving entry is test-only code**, served by the `[dev-dependencies]`
//! entry that cargo does not link into the product. `cargo tree -e normal -i
//! tinymemory-core` under the product feature set prints "nothing to print".
//!
//! Read the consequence carefully, because it inverts what this file used to
//! assume: **a non-empty list no longer means a linked engine.** Draining the
//! rest is still worth doing — a second door is a correctness problem whether
//! or not it ships — but it buys no bytes, and nobody should size it as though
//! it did.
//!
//! # This lint is a ratchet, not an invariant
//!
//! Same shape and same reasoning as [`super::bypass_allowlist_tests`], and as
//! `INTENTIONALLY_NOT_FORWARDED` in `scripts/lib/feature-forwarding.mjs`: the
//! current direct callers are enumerated in [`ALLOWED`] with a classification
//! and a reason each, and **that list may shrink but must never grow**. A lint
//! that was red on day one would be `#[ignore]`d within a week; a green ratchet
//! converges.
//!
//! # The classification, and why most of the list cannot move yet
//!
//! Each entry carries a [`Verdict`], which is the inventory the migration is
//! driven from:
//!
//! - [`Verdict::SeamExpressible`] — the existing `MemoryProvider` surface
//!   already covers this. These are the ones to migrate; a non-empty set here
//!   is a to-do list, not a steady state.
//! - [`Verdict::NeedsWiderSeam`] — the call wants something the thirteen
//!   capability families do not expose. **These are blocked upstream, not
//!   here.** `modules::registry` pins the TinyMemory module to a released,
//!   SHA-256-verified artifact (v1.0.1 at the time of writing), so a new bus
//!   method is a `tinymemory` release plus a registry re-pin before it is a
//!   host change. Adding the trait method alone would produce a driver that
//!   answers `Unsupported` — strictly worse than the direct call it replaced,
//!   because the failure moves from compile time to run time.
//! - [`Verdict::HostSide`] — not a driver call at all. Re-export shims,
//!   host-seam installation, and inert type imports. These are correct as they
//!   stand and are counted only so "deliberate" stays distinguishable from
//!   "forgotten".
//!
//! ## The concrete gaps, for whoever picks the upstream work up
//!
//! **This list was drained on 2026-08-23 and the shape of the problem changed.**
//! It used to enumerate four things the seam could not express — retrieval
//! filters, chunk reads, an entity-kind filter, and source listing — plus the
//! people domain and the `source_scope` task-local. Every one of those now has
//! a home:
//!
//! - **Retrieval filters, chunk reads, entity-kind search** — `MemoryRetrieval`
//!   (`fast_retrieve`, `cover_window`, `retrieve_source`, `retrieve_children`,
//!   `retrieve_leaves`, `recall_namespace_scored`, `search_entities`) and
//!   `MemoryChunks` (`list_chunks`, `get_chunk`, `chunk_detail`,
//!   `storage_kinds`, `chunk_embeddings`).
//! - **The people domain** — `MemoryPeople`, seven methods.
//! - **Profile/facets** — `MemoryProfile`, eleven methods, which
//!   `memory::guard`'s docs separately described as a missing "fourteenth
//!   family".
//! - **`source_scope`** — not a seam gap at all. It is host policy; it lives in
//!   `memory::source_scope`, and the scope crosses the bus as a `SourceScope`
//!   value on every scoped method.
//!
//! `ModuleMemoryProvider` implements all of these bar `as_episodic`, and
//! `MemoryGuard` wraps all fifteen families.
//!
//! **So what blocks the migration is release lag, not seam width.**
//! `modules::registry` pins a SHA-256-verified artifact, and the five families
//! above shipped in no release until v1.2.0. Before migrating a call site onto
//! one, check the *tag* rather than the vendored source — the submodule is
//! routinely ahead of what is pinned. A method the pinned artifact does not
//! serve answers `Unsupported` at run time, which is strictly worse than the
//! direct call it replaced.
//!
//! What genuinely has no bus representation yet, and is the next upstream ask:
//! the ingest `queue`, the `chat` runtime seam, the composio sync pipelines,
//! and **recency recall**. The first three live inside `tinymemory-core` and
//! would each need a design pass, not just a trait method.
//!
//! Recency recall is the subtle one, and it is worth spelling out because the
//! obvious migration is wrong. `MemoryRetrieval::recall_namespace_scored` looks
//! like the twin for `memory.recall_context` and `memory.recall_memories`, and
//! it is not:
//!
//! - `recall_namespace_scored` resolves to
//!   `query_namespace_hits_excluding_session(ns, query, limit, exclude)` — the
//!   **query-ranked** path.
//! - Both handlers call `recall_namespace_memories(ns, limit)` — a distinct
//!   **recency** path. `recall_namespace_context_data` is just that plus a
//!   rendered `context_text` wrapper.
//!
//! Passing an empty query to the scored method does not degrade to recency; it
//! runs the ranking path with nothing to rank against. The two share a prefix
//! (`load_documents_for_scope` + `kv_records_for_scope`) and diverge after it,
//! so the swap compiles, returns plausible hits, and quietly changes what the
//! user gets back. `memory.query_namespace` *is* safely expressible, because it
//! has a real query — pass `exclude_session_id: None` there to preserve the
//! current no-exclusion behaviour, since an RPC handler is not an agent turn.
//!
//! The upstream ask is a `RecallNamespaceRecent`-shaped method on
//! `MemoryRetrieval`.
//!
//! ## What the 2026-08-22 audit added to that list
//!
//! Draining `FacadeRevealed` turned 82 unexamined files into evidence, and it
//! widened the ask rather than narrowing it. Grouped by what blocks them, so
//! the upstream work can be sized per gap instead of per file:
//!
//! - **The engine handle itself** (~28 files) — `global::{init, client,
//!   client_if_ready}`, `store::{UnifiedMemory, MemoryClient, MemoryClientRef}`
//!   and `store::factories::create_memory`. These construct or hold the
//!   in-process engine. Nothing routes here: the seam has no door onto a live
//!   client, and it should not grow one — this is `memory::binding`'s job, and
//!   the ask is that every caller take the binding's provider instead.
//! - **Chunk writes and transactions** (~21 files) —
//!   `store::chunks::store::{with_connection, upsert_chunks,
//!   upsert_staged_chunks_tx, get_or_init_connection}`, plus `store::{fts5,
//!   segments, profile, events, content}`. `MemoryChunks` is a read family;
//!   this is the write half, and `with_connection` hands out a SQLite handle,
//!   which no engine-neutral contract can promise. **Moving these subsystems
//!   behind the bus is the only shape that keeps a supermemory/mem0/cognee
//!   driver implementable** — a contract with `with_connection` in it is a
//!   SQLite contract wearing a trait.
//! - **Host policy reached through the engine crate** — **drained.** The
//!   scrubbers (`store::safety::{sanitize_text, sanitize_json,
//!   has_likely_secret}`), `util::redact::redact`, `source_scope::*` and the
//!   Obsidian vault-registration probe (`store::content::obsidian_registry`)
//!   all live host-side now, in `memory::safety`, `util::redact`,
//!   `memory::source_scope` and `memory::obsidian_registry`. The route each
//!   took is the shape to reuse, and it is **not** "move it to
//!   `tinymemory-api`": a scrubber costs `regex` + `serde_json`,
//!   `util::redact` costs `sha2`, and the vault probe costs `dirs` +
//!   `serde_json`, in a crate whose whole point is that a caller can depend on
//!   it and compile almost nothing — and `source_scope` is a
//!   `tokio::task_local`, which would put tokio there too. None of the four is
//!   contract vocabulary — nothing crosses the bus as a `SanitizationReport`,
//!   a log hash or a `VaultRegistration`, and "is my content root a registered
//!   Obsidian vault" is not a capability a second driver would answer
//!   differently — so each is simply the host's, with the engine keeping its
//!   own copy for its own callers. Independent copies are the design: neither
//!   side reads the other's output.
//! - **The re-embed queue** (~8 files) — `queue::{start, store, types,
//!   ensure_reembed_backfill, requeue_failed_after_provider_change,
//!   drain_until_idle, wake_workers, backfill_in_progress}`. No family.
//! - **Engine-shaped integration internals** (~11 files) —
//!   `tinycortex::{memory_config_from, run_composio_connection,
//!   load_composio_sync_state, HostSyncAdapter, CodingSession*}`. Named after
//!   the engine, so no engine-neutral family can express them as they stand.
//! - **Engine-owned types** — `store::trees::types::TreeKind`, and
//!   `store::{NamespaceDocumentInput, NamespaceRetrievalContext,
//!   GraphRelationRecord}` (`store::chunks::types::SourceKind`/`SourceRef`
//!   were on this list and are **done** — see below). A type import links the
//!   crate exactly as a call does, so the shed needs these in
//!   `tinymemory-api`. `MemoryCategory`/`MemoryEntry`/`MemoryTaint` already
//!   are — `tinymemory_core::traits` re-exports them — so those call sites can
//!   name the contract today.
//!
//!   `rpc_models` was on this list and is **done**: all forty-five types were
//!   named by this host and by nothing inside `tinymemory`, so they moved to
//!   `memory::rpc_models` rather than into the contract. That is the shape to
//!   look for first in what remains — a type the engine crate defines but only
//!   the host uses does not need a contract to live in, it needs to come home.
//!   `SourceKind`/`SourceRef` were called out here as emphatically **not**
//!   such a case, on the grounds that the engine path resolved to a
//!   `tinycortex-api` type distinct from the contract's. That is no longer
//!   true: `tinycortex-api` is a deprecated re-export of `tinymemory-bus` and
//!   the two paths resolve to the **same item**, verified by a compile-time
//!   identity probe and then by repointing every call site. Prefer
//!   `tinymemory_api::chunks::…` in new code. The general warning still holds
//!   for *other* near-identical pairs — probe before assuming, either way.
//! - **Chat, ingest pipeline and preferences** (~12 files) —
//!   `chat::{ChatProvider, build_chat_provider, test_override}`,
//!   `ingest_pipeline::{ingest_chat, ingest_document_with_scope}`,
//!   `preferences::{STANDING_PREFS_LIMIT, load_general_preferences,
//!   recall_situational_preferences}`.
//!
//! The order that follows from this: relocate the pure helpers and types to
//! `tinymemory-api` (no bus surface, no release coupling), then move the
//! queue and chunk-write subsystems behind the module, and only then can the
//! handle-holding callers take the binding's provider and the crate leave the
//! build. Nothing here is a host-side routing pass, which is what the original
//! scope assumed.
//!
//! # Known weaknesses, stated rather than hidden
//!
//! - **One needle, two crates — and #5560 sheds both.** [`NEEDLE`] is
//!   `tinymemory_core::` alone, but `tinycortex` is a direct dependency of this
//!   crate in its own right, not something reached through the engine crate. So
//!   repointing a file from `tinymemory_core::x` to `tinycortex::x` clears its
//!   entry here while leaving an engine linked, and the ratchet reads as
//!   progress. **That is not a migration; it is the lint losing sight of the
//!   file.** `memory::tree::health` moved that way legitimately — the taxonomy
//!   was always `tinycortex`'s and the engine crate only re-exported it — and
//!   `memory::tools::flavour` was a `tinycortex` caller this lint never saw at
//!   all until it moved onto `MemoryTree::flavour_profile`. Before concluding
//!   the crates have left the build, run the scan for **both** spellings; at
//!   the time of writing `tinycortex::` finds one production file
//!   (`src/bin/library_profile/scenarios/memory_ingest.rs`) and it is already
//!   listed below for the other needle.
//! - **The lint sees text, not types.** A reference reached through a
//!   re-export under another name is invisible to it — and the memory tree is
//!   full of those on purpose: `memory/mod.rs` re-exports twenty-five engine
//!   modules, and ~687 `memory::store::…` / `memory::tree::…` paths elsewhere
//!   resolve into the crate through them. **This lint deliberately does not
//!   count those.** It counts the sites that *name* the crate, because those
//!   are the ones a migration edits. The re-export surface is a separate,
//!   larger problem tracked in the issue, and pretending this number covers it
//!   would be the worst outcome.
//! - **By-path test files are out of scope** (`*_tests.rs`, `tests.rs`,
//!   `test_support/`), matching the sibling lint. Several inline
//!   `#[cfg(test)]` modules do name the crate (`query::drill_down`,
//!   `query::fetch_leaves`, `query::query_source` each assert a tool's result
//!   against a direct engine call); those files are listed, and the entry says
//!   so.
//! - **Comment lines are skipped**, so the many doc comments that reference
//!   `tinymemory_core::…` by path do not inflate the count.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Why a file may name the engine crate today.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Verdict {
    /// The existing `MemoryProvider` surface covers this. Migrate it.
    SeamExpressible,
    /// Blocked on a wider bus surface, which means an upstream `tinymemory`
    /// release and a `modules::registry` re-pin.
    NeedsWiderSeam,
    /// Not a driver call: a re-export shim, host-seam installation, or an
    /// inert type import.
    HostSide,
    /// The file's engine dependency **predates this branch** and was hidden
    /// behind `memory/mod.rs`'s re-export facade; deleting that facade made it
    /// textually visible to this lint without changing what the file does.
    ///
    /// It is a deliberately unflattering label. These are not audited, and the
    /// bucket exists so that "nobody has looked at this yet" cannot be mistaken
    /// for one of the three considered verdicts above. Draining it means
    /// re-classifying each entry as one of those three, not deleting the
    /// variant.
    ///
    /// **Drained 2026-08-22.** All 82 entries were audited into the three
    /// considered verdicts; none turned out to be [`Verdict::SeamExpressible`],
    /// which is the finding rather than a formality — every one of them is
    /// blocked on a contract the module does not yet expose, so the remaining
    /// #5560 work is upstream in `tinymemory` and not a routing pass here. The
    /// variant is kept rather than removed for the reason it was added: if a
    /// re-export facade grows back and hides engine users again, the label for
    /// them already exists and already says what it means.
    #[allow(
        dead_code,
        reason = "drained 2026-08-22; retained as the landing spot if a facade regrows"
    )]
    FacadeRevealed,
}

/// The literal this lint searches for. A single needle, deliberately: the
/// question is "does this file name the engine crate", not "which item".
const NEEDLE: &str = "tinymemory_core::";

/// `(repo-relative path, verdict, why it names the engine today)`.
///
/// Adding an entry is a decision, not a way to silence the lint. Sorted by
/// path — [`scan`] returns a `BTreeSet`, so keeping the literal in the same
/// order makes diffs readable.
const ALLOWED: &[(&str, Verdict, &str)] = &[
    // ── Revealed by deleting the `memory/mod.rs` re-export facade ───────────
    //
    // This lint was calibrated against a tree where `memory/mod.rs` re-exported
    // ~24 engine names, so a file writing `crate::openhuman::memory::UnifiedMemory`
    // did not match the `tinymemory_core::` needle. Those files were direct
    // engine users the whole time; the facade just spelled the dependency
    // differently. Deleting it — so that `grep tinymemory_core` *is* the
    // inventory — is what surfaced them, and the count of real engine
    // dependencies did not grow by one.
    //
    // Audited individually on 2026-08-22 and re-classified out of
    // `FacadeRevealed`; each entry now names the symbols it actually reaches
    // for, so the verdict is checkable against the code rather than taken on
    // trust. The audit's finding is that none of them is `SeamExpressible`.
    (
        "src/bin/library_profile/scenarios/cold_phases.rs",
        Verdict::NeedsWiderSeam,
        "holds or boots the in-process engine handle (store::MemoryClient); driver construction belongs to memory::binding and the seam has no door onto the live client",
    ),
    (
        "src/bin/library_profile/scenarios/memory_ingest.rs",
        Verdict::NeedsWiderSeam,
        "engine-internal ingest pipeline entry (ingest_pipeline::ingest_chat, queue::drain_until_idle); the ingest family covers documents and chat, not the scope-carrying pipeline variants",
    ),
    (
        "src/openhuman/agent/harness/archivist/recap.rs",
        Verdict::NeedsWiderSeam,
        "the engine-side chat provider seam (chat::test_override) plus the engine fold it scopes (tree::summarise::{summarise, SummaryInput, SummaryContext}, tree::tree::TreeKind), all named only from the `#[cfg(test)]` recap arm: the deterministic provider those tests install is a task-local inside this binary's copy of the engine, which a module in its own process cannot see. Named engine-direct since the memory::tree shims were deleted. The production fold is MemoryTree::summarise",
    ),
    (
        "src/openhuman/agent/harness/archivist/test_constructors.rs",
        Verdict::NeedsWiderSeam,
        "the engine-side chat provider seam (chat::ChatProvider); MemoryIngest has no provider-override door",
    ),
    (
        "src/openhuman/agent/harness/archivist/tree_ingest.rs",
        Verdict::NeedsWiderSeam,
        "reaches engine storage below the contract (store::fts5, store::segments::ConversationSegment, ingest_pipeline); MemoryChunks is read-only (list_chunks/get_chunk/chunk_detail/storage_kinds/chunk_embeddings) with no write or transaction door",
    ),
    (
        "src/openhuman/agent/harness/archivist/types.rs",
        Verdict::NeedsWiderSeam,
        "the engine-side chat provider seam (chat::ChatProvider); MemoryIngest has no provider-override door",
    ),
    (
        "src/openhuman/channels/tests/memory.rs",
        Verdict::NeedsWiderSeam,
        "holds or boots the in-process engine handle (store::UnifiedMemory); driver construction belongs to memory::binding and the seam has no door onto the live client",
    ),
    (
        "src/openhuman/integrations/composio/ops/mod.rs",
        Verdict::NeedsWiderSeam,
        "holds or boots the in-process engine handle (store::MemoryClient); driver construction belongs to memory::binding and the seam has no door onto the live client",
    ),
    (
        "src/openhuman/memory/read_rpc/mod.rs",
        Verdict::NeedsWiderSeam,
        "one `#[cfg(test)]` re-export of store::chunks::store::with_connection, the raw SQLite door the read_rpc tests assert written rows through — asserting storage rather than re-reading through the handler under test. MemoryChunks is read-only (list_chunks/get_chunk/chunk_detail/storage_kinds/chunk_embeddings) with no transaction door, and a contract that had one would be a SQLite contract wearing a trait. Nothing production-side in read_rpc names the engine: SourceKind is tinymemory_api::chunks::SourceKind, and wipe_all/delete_source/flush_source_tree are purge_all, forget_matching and Tree::flush_source_tree",
    ),
    // ── Re-export shims: `pub use tinymemory_core::<domain>::*;` ────────────
    //
    // **Drained.** Four of these existed — `tree/mod.rs`, `tree/health/mod.rs`,
    // `tree/tree/mod.rs` and `tree/tree_runtime/mod.rs` — carrying the
    // historical-path aliases `memory/mod.rs` documented. The first three had
    // no production consumer left, only tests; those tests name the engine
    // crates directly now (served by the `[dev-dependencies]` tinymemory-core
    // entry) and the globs went with them.
    //
    // `tree_runtime` was the last, and the only one that was still production-
    // live: five `tree_summarizer_*` RPC handlers, the `tree-summarizer` CLI,
    // `memory::ops::learn` and the channels-startup subscriber ran the markdown
    // time tree in-process through it. It is gone because the seam grew the six
    // doors it needed — `RuntimeBufferWrite`, `RuntimeReadNode`,
    // `RuntimeReadChildren`, `RuntimeTreeStatus`, `RuntimeSummarize`,
    // `RuntimeRebuild` — in tinymemory PR #123 (contract 4.0). That is the
    // shape every remaining `NeedsWiderSeam` entry below is waiting for, and
    // the first one to complete the loop: upstream door, host migration,
    // entry deleted.
    //
    // ── Host-seam installation: the host handing itself TO the engine ───────
    //
    // The direction of these is inbound, not outbound: they install embedding /
    // chat / config / NLP / scheduler / shutdown / error-reporting callbacks
    // into the in-process engine. `modules/memory_host.rs` is the same seam
    // served over the bus. They are what an embedded engine needs, and they
    // are the last thing to remove, not the first.
    (
        "src/openhuman/memory/host_impls.rs",
        Verdict::HostSide,
        "installs the seven host seams (embedding, chat, config, nlp, scheduler gate, shutdown, error reporter) into an in-process engine, and the whole module sits behind the default-ON/product-OFF `memory-engine-seams` feature since #5560 — it is listed here only because this lint scans source text and does not track cfg. A feature rather than #[cfg(test)] because a tests/ integration target links this crate as an ordinary dependency, where cfg(test) is false and the module would be invisible however the engine is declared, and two dozen of them install these seams. Its production callers are gone: runtime::context, memory_cli and agent::debug install memory::host::install_memory_event_sink() instead, which is a tinymemory-api seam (still a normal dependency) with a live host-side publisher in memory::sync::composio::bus. That split is load-bearing — tinymemory_api::events::publish drops silently when unwired, so folding the sink in here would have removed a live event path with no error anywhere. What still needs these installs is install_for_tests, whose ~90 callers stand up a real in-process engine from the [dev-dependencies] entry. The seams themselves are also served for the LOADED module by modules/memory_host.rs, over the module's own inbound interfaces — a separate mechanism, since a cdylib has its own statics and never saw what was set here",
    ),
    // ── Retrieval: filters the seam's tree family has no room for ───────────
    // ── Agent tools: chunk reads, source listing, people, source scope ──────
];

/// True for source files the lint deliberately does not scan.
///
/// By-path only, matching [`super::bypass_allowlist_tests`] — see that module
/// for why inline `#[cfg(test)]` blocks are left in scope rather than
/// brace-tracked.
fn is_test_path(path: &Path) -> bool {
    if path.components().any(|c| c.as_os_str() == "test_support") {
        return true;
    }
    match path.file_name().and_then(|n| n.to_str()) {
        Some(name) => name == "tests.rs" || name.ends_with("_tests.rs"),
        None => false,
    }
}

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") && !is_test_path(&path) {
            out.push(path);
        }
    }
}

/// Every repo-relative path in this crate's `src` that names the engine crate
/// outside a comment.
fn scan() -> BTreeSet<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    collect_rs_files(&root.join("src"), &mut files);

    let mut found = BTreeSet::new();
    for path in &files {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        for line in text.lines() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            if line.contains(NEEDLE) {
                found.insert(rel.clone());
                break;
            }
        }
    }
    found
}

fn allowed_set() -> BTreeSet<String> {
    ALLOWED
        .iter()
        .map(|(path, _, _)| (*path).to_string())
        .collect()
}

fn render(paths: impl IntoIterator<Item = String>) -> String {
    paths.into_iter().map(|p| format!("\n  {p}")).collect()
}

/// A scanner that silently found nothing would turn every other test here into
/// a rubber stamp, so refuse to pass vacuously.
///
/// `memory/host_impls.rs` is the most stable pin available now that the
/// `memory/mod.rs` re-export block has been drained: it installs the seven host
/// seams, so it names the crate by construction for as long as the engine is
/// linked at all. If the scanner stops seeing it, the scanner is broken — fix
/// it, do not relax this assertion.
#[test]
fn direct_reference_scanner_is_not_vacuous() {
    let found = scan();
    let allowed = allowed_set();

    // The canary only holds while the allowlist still names it. Asserting it
    // unconditionally would turn the last migration in this file into a
    // failure, which is backwards: draining the list is the goal.
    if allowed.contains("src/openhuman/memory/host_impls.rs") {
        assert!(
            found.contains("src/openhuman/memory/host_impls.rs"),
            "scanner found no direct engine reference in memory/host_impls.rs, which installs \
             the host seams; the scanner is broken"
        );
    }

    // The real vacuity risk is `scan()` silently returning nothing — a broken
    // walk, a moved `src/`, a needle that stopped matching — which would turn
    // `no_new_files_call_the_engine_directly` into a rubber stamp. Pin it to
    // the allowlist rather than to a literal, so the assertion stays true as
    // the list drains instead of having to be hand-edited on every migration.
    //
    // It was a literal (`found.len() > 20`) until 2026-08-31, by which point
    // ALLOWED itself had shrunk to exactly 20 — and because the two difference
    // tests below force `found == allowed`, `20 > 20` made this test
    // unsatisfiable on `main`. A ratchet that cannot pass is not a ratchet.
    assert!(
        !allowed.is_empty() || found.is_empty(),
        "the allowlist is empty but the scanner still found {} file(s): {}",
        found.len(),
        render(found.iter().cloned())
    );
    assert!(
        allowed.is_empty() || !found.is_empty(),
        "scanner found nothing while the allowlist still names {} file(s); the scanner is broken",
        allowed.len()
    );
}

/// **The ratchet.** A new file naming `tinymemory_core::` fails here.
///
/// If the new call is genuinely unavoidable, add it to [`ALLOWED`] with a
/// [`Verdict`] and a reason. If it is not, route it through
/// `CoreContext::memory()` and the `MemoryProvider` seam.
#[test]
fn no_new_files_call_the_engine_directly() {
    let found = scan();
    let allowed = allowed_set();
    let unexpected: Vec<String> = found.difference(&allowed).cloned().collect();
    assert!(
        unexpected.is_empty(),
        "new direct `tinymemory_core::` reference(s) — route these through the MemoryProvider seam, \
         or add them to ALLOWED with a Verdict and a reason:{}",
        render(unexpected)
    );
}

/// The staleness half. An allowlist that outlives its entries rots into dead
/// strings that document nothing — the same failure `INTENTIONALLY_NOT_FORWARDED`
/// guards against. A migrated file must be *removed* from the list, so the
/// count is always the real one.
#[test]
fn allowlist_has_no_stale_entries() {
    let found = scan();
    let allowed = allowed_set();
    let stale: Vec<String> = allowed.difference(&found).cloned().collect();
    assert!(
        stale.is_empty(),
        "ALLOWED names file(s) that no longer reference the engine — delete these entries so the \
         ratchet reflects reality:{}",
        render(stale)
    );
}

/// Every entry carries a reason, and no path is listed twice. A blank reason is
/// an allowlist entry that documents nothing, which is what the list exists to
/// prevent.
#[test]
fn allowlist_entries_are_well_formed() {
    let mut seen = BTreeSet::new();
    for (path, _, reason) in ALLOWED {
        assert!(
            !reason.trim().is_empty(),
            "{path} is allowlisted with no reason"
        );
        assert!(
            seen.insert(*path),
            "{path} is listed twice; one entry per file"
        );
    }
}

/// The migration to-do list must be empty, and stay empty by being *worked*
/// rather than re-labelled.
///
/// A [`Verdict::SeamExpressible`] entry says "the seam already covers this and
/// nobody moved it". That is a bug with a known fix, so it fails here rather
/// than sitting in a list nobody reads. Downgrading an entry to
/// [`Verdict::NeedsWiderSeam`] to silence this is the one edit that would make
/// the lint lie — [`no_new_files_call_the_engine_directly`] would still pass,
/// and the gap would vanish from view.
#[test]
fn nothing_is_left_migratable() {
    let pending: Vec<&str> = ALLOWED
        .iter()
        .filter(|(_, verdict, _)| *verdict == Verdict::SeamExpressible)
        .map(|(path, _, _)| *path)
        .collect();
    assert!(
        pending.is_empty(),
        "these files can already be expressed through MemoryProvider and should be migrated: {pending:?}"
    );
}

/// The blocked set is the upstream ask, so it must be non-empty for as long as
/// any file still names the engine crate — and empty when none does.
///
/// **This test was written to be self-proving and was not.** Its premise was
/// that "the crate is dropped" and "`ALLOWED` empties" are the same event, so
/// that the day one happened the other would force the module docs above to be
/// rewritten. #5560 falsified that on 2026-08-31: `tinymemory-core` left
/// `[dependencies]` with **nine entries still listed**, and this assertion did
/// not move, because [`scan`] reads source text and every surviving entry is
/// test-only code that the `[dev-dependencies]` entry still compiles.
///
/// The rewrite happened anyway — by hand, not because a test demanded it. Do
/// not restore the old wording, and do not add an assertion tying this list to
/// the manifest: the two are genuinely independent now, and a lint that claimed
/// otherwise would fail for a build that is correct.
///
/// What it still buys is the honest half: a list that empties while files
/// remain, or files that remain while the list empties, is a broken scanner.
#[test]
fn the_blocked_set_matches_the_engine_still_being_linked() {
    let blocked = ALLOWED
        .iter()
        .filter(|(_, verdict, _)| *verdict == Verdict::NeedsWiderSeam)
        .count();
    let host_side = ALLOWED
        .iter()
        .filter(|(_, verdict, _)| *verdict == Verdict::HostSide)
        .count();
    assert!(
        blocked > 0 || host_side > 0,
        "nothing references tinymemory-core any more — drop the remaining \
         [dev-dependencies] entry from Cargo.toml and rewrite this module's docs. \
         The [dependencies] entry, the cargo-machete `ignored` list and \
         scripts/kernel-floor.limits were all settled in #5560, when the crate left \
         the product build with this list still non-empty"
    );
}
