//! Enforcement lint: the set of production files that reach the memory driver
//! **around** the kernel guard must not grow.
//!
//! # This test does NOT claim the guard is the only path to the driver
//!
//! It is not, and this file exists to make that measurable rather than to
//! pretend otherwise. `MemoryClient` still hands out raw, undecoratable
//! handles, all `pub(crate)` and all with live production callers:
//!
//! - `profile_store()` (`memory/store/client.rs`) — a typed `ProfileStore`
//!   over the profile/facet tables. `profile_conn()`, the raw
//!   `Arc<Mutex<rusqlite::Connection>>` it used to hand out, is now
//!   `pub(in crate::openhuman::memory)` with a single in-family caller, so
//!   every SQL statement against `user_profile` lives inside the memory
//!   family and the compiler keeps it there. **This did not put the profile
//!   tables under the guard**: the contract has no profile/facet capability
//!   family, so these reads and writes still run beneath all seven policy
//!   steps. The `.profile_store(` needle exists to keep that fact counted
//!   rather than renamed away.
//! - `memory_handle()` (`memory/store/client.rs`) — a raw `Arc<dyn Memory>`.
//!   The contract has no `Arc<dyn Memory>` door, so consumers that must satisfy
//!   a foreign trait (tinyflows, the agent-experience store) still take it.
//! - `get_document()` (`memory/store/client.rs`) — a read-one escape hatch that
//!   is driver-only by contract but not by visibility.
//!
//! What this test buys is a **ratchet**, not an invariant: the current bypasses
//! are enumerated in [`ALLOWED`] with a reason each, and that list may shrink
//! but never grow. That is deliberately weaker than "impossible to skip by
//! construction", and the weakness is the point — a lint that was red on day
//! one would be `#[ignore]`d within a week and never come back, whereas a green
//! ratchet converges. An enforcement test that overstates its own guarantee is
//! worse than none, because it stops people looking.
//!
//! Same pattern and same reasoning as `INTENTIONALLY_NOT_FORWARDED` in
//! `scripts/lib/feature-forwarding.mjs`, including its staleness half — which
//! is what stops an allowlist rotting into dead strings.
//!
//! # Known weaknesses, stated rather than hidden
//!
//! - **The lint sees text, not types.** A bypass reached through a re-export
//!   under a different name is invisible to it. Substring needles also
//!   over-match: an unrelated future `.get_document(` would trip this. That
//!   failure direction is the correct one — a false positive costs one
//!   allowlist line with a reason, a false negative costs a silent bypass.
//!   One needle has a compiler backstop: `.unguarded_provider(` is
//!   `pub(in crate::openhuman::memory)`, so even a re-export under another name
//!   cannot carry it outside the memory family. It was also *named* for the
//!   lint — `.provider(` would have over-matched `TaskSourceFilter::provider()`
//!   and `ModelRef::provider()` in four unrelated production files, costing six
//!   allowlist entries that document nothing.
//! - **By-path test files are out of scope** (`*_tests.rs`, `tests.rs`,
//!   `test_support/`). Driver tests construct drivers; that is what a driver
//!   test *is*. Allowlisting them would add ~25 entries that can never shrink
//!   and would churn on every new driver test — the exact rot this lint fights.
//! - **Inline `#[cfg(test)] mod tests` blocks are NOT stripped.** Brace-tracking
//!   Rust source with a line scanner is fragile, and getting it wrong silently
//!   *hides* production sites. Three files are therefore allowlisted for a
//!   match that lives only in an inline test module; each says so.
//! - **`app/src-tauri/` is not scanned.** It links `openhuman_core` with
//!   `default-features = false` and cannot name `pub(crate)` items at all, so
//!   there is nothing there to scan. The omission is deliberate, not an
//!   oversight.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Call shapes that hand out an **unguarded** door into memory, each with why
/// reaching it around the guard matters.
///
/// Substring needles, not regexes, and deliberately path-*suffixed*: the same
/// call is written `memory::global::client_if_ready()`,
/// `crate::openhuman::memory::global::client_if_ready()` and
/// `super::super::global::client_if_ready()` in this tree, so anchoring on an
/// absolute path would miss the third.
///
/// `global::init(` is deliberately absent. It binds a workspace; it does not
/// read or write memory, and every call site is a login / user-switch / boot /
/// CLI-entry lifecycle event. See `docs/specs/memory-guard-allowlist.md`.
const BYPASS_PATTERNS: &[(&str, &str)] = &[
    (
        "active_memory_client(",
        "resolves a MemoryClientRef with no policy decorator",
    ),
    (
        "global::client_if_ready(",
        "process-global MemoryClient, no policy decorator",
    ),
    (
        "global::client(",
        "process-global MemoryClient, no policy decorator",
    ),
    (
        ".profile_conn(",
        "raw rusqlite connection — undecoratable by construction",
    ),
    (
        ".profile_store(",
        "typed profile store — the profile/facet tables have no capability family, so these reads and writes still skip the guard's seven steps",
    ),
    (
        ".memory_handle(",
        "raw Arc<dyn Memory> — bypasses the MemoryClient API surface",
    ),
    (
        ".get_document(",
        "pub(crate) read-one escape hatch, driver-only by contract",
    ),
    (
        "EmbeddedMemoryProvider::new(",
        "direct driver construction — must go through binding::for_workspace",
    ),
    (
        "NullMemoryProvider::new(",
        "direct driver construction — must go through binding::for_workspace",
    ),
    (
        "MemoryClient::from_workspace_dir(",
        "direct engine construction; risks a second ingestion worker on one store",
    ),
    (
        "binding::for_workspace(",
        "raw MemoryBinding — the guard is what should be handed out",
    ),
    (
        ".memory_binding(",
        "raw MemoryBinding off CoreContext instead of CoreContext::memory()",
    ),
    (
        ".unguarded_provider(",
        "raw Arc<dyn MemoryProvider> off a MemoryBinding — skips the guard entirely",
    ),
];

/// `(repo-relative path, pattern, why this file may bypass today)`.
///
/// Adding an entry is a decision, not a way to silence the lint. The reason
/// string is what a future reviewer relies on to tell "deliberate" from
/// "forgotten". **This list may SHRINK. It must never GROW.**
///
/// Sorted by path, then pattern — [`scan`] returns a `BTreeSet`, so keeping the
/// literal in the same order makes diffs readable.
const ALLOWED: &[(&str, &str, &str)] = &[
    // ── Standalone binaries: their own process, no ambient CoreContext ──
    (
        "src/bin/library_profile/scenarios/cold_phases.rs",
        "MemoryClient::from_workspace_dir(",
        "profiling harness; boots its own client outside the guard's process model",
    ),
    (
        "src/bin/slack_backfill.rs",
        "global::client_if_ready(",
        "standalone backfill binary; boots its own client, no CoreContext",
    ),
    // ── Metadata-only reads: driver identity, never memory content ──
    (
        "src/core/cli_capability.rs",
        "binding::for_workspace(",
        "reads driver_id + advertised capabilities only (what memory.provider_status already reports); no CoreContext exists on a CLI invocation, so there is no guard to route through",
    ),
    // subsystems_cli.rs no longer binds directly: it delegates to
    // memory_subsystem_status, whose binding resolution lives in
    // memory/ops/provider.rs (allowlisted above).
    // ── The bind site itself: it produces the guard ──
    (
        "src/core/runtime/context.rs",
        ".memory_binding(",
        "CoreContext owns the binding; memory() is the guarded accessor built from it",
    ),
    (
        "src/core/runtime/context.rs",
        "binding::for_workspace(",
        "the per-workspace bind site — guarding it would be a cycle",
    ),
    // ── Needs a concrete engine type the contract does not expose ──
    (
        "src/openhuman/agent/experience/ops.rs",
        ".memory_handle(",
        "AgentExperienceStore::new takes Arc<dyn Memory>; no contract door for it",
    ),
    (
        "src/openhuman/agent/experience/ops.rs",
        "global::client_if_ready(",
        "same store; also builds a per-profile UnifiedMemory the binding cannot model",
    ),
    (
        "src/openhuman/agent/harness/session/builder/factory.rs",
        ".memory_handle(",
        "session builder needs Arc<dyn Memory>; no contract door for it",
    ),
    // ── Unguarded (but no longer raw) profile/facet access ──
    (
        "src/openhuman/agent/learning/schemas.rs",
        ".profile_store(",
        "typed profile/facet reads; the contract has no profile family, so still unguarded",
    ),
    (
        "src/openhuman/agent/learning/schemas.rs",
        "global::client_if_ready(",
        "resolved only to reach profile_store() on the line below",
    ),
    (
        "src/openhuman/agent/learning/startup.rs",
        "MemoryClient::from_workspace_dir(",
        "inline #[cfg(test)] module only; the scanner does not brace-track test blocks",
    ),
    (
        "src/openhuman/agent/learning/startup.rs",
        ".profile_store(",
        "typed facet bootstrap; the contract has no profile family, so still unguarded",
    ),
    (
        "src/openhuman/agent/learning/tools.rs",
        ".profile_store(",
        "typed facet read from an agent tool; the contract has no profile family",
    ),
    (
        "src/openhuman/agent/learning/tools.rs",
        "global::client_if_ready(",
        "resolved only to reach profile_store() on the line below",
    ),
    // ── Flows: foreign trait shapes and a test-override seam ──
    (
        "src/openhuman/flows/bus.rs",
        ".memory_handle(",
        "resolve_memory() -> Option<Arc<dyn Memory>>; no contract door for it",
    ),
    (
        "src/openhuman/flows/bus.rs",
        "active_memory_client(",
        "carries a #[cfg(test)] memory_override seam the guard would bypass",
    ),
    (
        "src/openhuman/flows/ops.rs",
        "active_memory_client(",
        "clear_namespace has no contract method; plus a memory_client_override test seam",
    ),
    (
        "src/openhuman/flows/tinyflows/memory_adapter.rs",
        ".memory_handle(",
        "returns Arc<dyn Memory> to satisfy a tinyflows engine trait",
    ),
    (
        "src/openhuman/flows/tinyflows/memory_adapter.rs",
        "active_memory_client(",
        "same adapter; the tinyflows trait names the engine type, not the contract",
    ),
    // ── Composio integration: &MemoryClientRef parameter shape ──
    (
        "src/openhuman/integrations/composio/ops/memory_cleanup.rs",
        "MemoryClient::from_workspace_dir(",
        "cleanup runs off a config workspace with no live binding",
    ),
    (
        "src/openhuman/integrations/composio/schemas.rs",
        "global::client_if_ready(",
        "passes &MemoryClientRef into user_scopes::save; the contract has no such shape",
    ),
    // ── The driver and the binding: guarding these would be a cycle ──
    (
        "src/openhuman/memory/binding.rs",
        "EmbeddedMemoryProvider::new(",
        "this is the construction path the lint protects",
    ),
    (
        "src/openhuman/memory/binding.rs",
        "NullMemoryProvider::new(",
        "this is the construction path the lint protects (fail-closed fallback)",
    ),
    (
        "src/openhuman/memory/driver/embedded/documents.rs",
        ".get_document(",
        "this IS the driver — the escape hatch exists for exactly this call",
    ),
    (
        "src/openhuman/memory/driver/embedded/mod.rs",
        ".memory_handle(",
        "this IS the driver; it owns the engine handle by definition",
    ),
    (
        "src/openhuman/memory/driver/embedded/mod.rs",
        "EmbeddedMemoryProvider::new(",
        "the driver's own constructor",
    ),
    (
        "vendor/tinymemory/core/src/global.rs",
        "MemoryClient::from_workspace_dir(",
        "the process-global slot itself; it is what global::client hands out",
    ),
    (
        "src/openhuman/memory/guard/families.rs",
        ".get_document(",
        "the guard's own documents decorator forwarding to the inner family",
    ),
    // ── Handlers with no typed contract twin (see the allowlist doc, §D) ──
    (
        "src/openhuman/memory/ops/documents.rs",
        "active_memory_client(",
        "namespace/doc/context handlers answer untyped Value shapes with no contract twin",
    ),
    (
        "src/openhuman/memory/ops/guard.rs",
        "binding::for_workspace(",
        "the guarded resolver; this is where the binding becomes a guard",
    ),
    (
        "src/openhuman/memory/ops/helpers.rs",
        "active_memory_client(",
        "defines active_memory_client — the unguarded twin of ops::guard",
    ),
    (
        "src/openhuman/memory/ops/helpers.rs",
        "global::client_if_ready(",
        "same definition site",
    ),
    (
        "src/openhuman/memory/ops/kv_graph.rs",
        "active_memory_client(",
        "kv_get/kv_delete/graph_* have no contract twin or a lossy conversion",
    ),
    (
        "src/openhuman/memory/ops/learn.rs",
        "active_memory_client(",
        "list_namespaces() -> Vec<String> vs the contract's Vec<NamespaceSummary>",
    ),
    (
        "src/openhuman/memory/ops/learn.rs",
        "global::client(",
        "inline #[cfg(test)] module only; the scanner does not brace-track test blocks",
    ),
    (
        "src/openhuman/memory/ops/provider.rs",
        ".memory_binding(",
        "reports driver status; it is about the binding, not about reading memory",
    ),
    (
        "src/openhuman/memory/ops/provider.rs",
        ".unguarded_provider(",
        "health probe on the bound driver; a liveness probe is not product code",
    ),
    (
        "src/openhuman/memory/ops/provider.rs",
        "binding::for_workspace(",
        "same status surface",
    ),
    (
        "src/openhuman/memory/ops/sync.rs",
        "global::client_if_ready(",
        "ingestion_state().snapshot() — queue telemetry, absent from the contract",
    ),
    (
        "src/openhuman/memory/ops/sync.rs",
        "global::client(",
        "inline #[cfg(test)] module only; the scanner does not brace-track test blocks",
    ),
    (
        "src/openhuman/memory/ops/tool_memory.rs",
        ".memory_handle(",
        "open_store() still serves the four handlers with no contract twin",
    ),
    (
        "src/openhuman/memory/ops/tool_memory.rs",
        "active_memory_client(",
        "tool_rule_put/get/*_json/*_for_prompt have no contract equivalent",
    ),
    (
        "vendor/tinymemory/core/src/store/client.rs",
        ".profile_conn(",
        "sole in-family call; wraps the raw handle in ProfileStore. profile_conn is pub(in crate::openhuman::memory), so the compiler — not this lint — is the primary enforcement",
    ),
    // ── Composio memory sync: profile_store + &MemoryClientRef ──
    (
        "vendor/tinymemory/core/src/sync/composio/providers/profile.rs",
        ".profile_store(",
        "typed profile writes; the contract has no profile family, so still unguarded",
    ),
    (
        "vendor/tinymemory/core/src/sync/composio/providers/profile.rs",
        "global::client_if_ready(",
        "resolved only to reach profile_store()",
    ),
    (
        "vendor/tinymemory/core/src/sync/composio/providers/types.rs",
        "MemoryClient::from_workspace_dir(",
        "provider trait takes &MemoryClientRef; the contract has no such shape",
    ),
    (
        "vendor/tinymemory/core/src/sync/composio/providers/types.rs",
        "global::client_if_ready(",
        "same provider trait shape",
    ),
    (
        "vendor/tinymemory/core/src/sync/composio/providers/user_scopes.rs",
        "global::client_if_ready(",
        "same provider trait shape",
    ),
    // ── Golden-workspace fixture seeder (test infrastructure) ──
    //
    // `memory::store::golden` is the engine behind the `memory_golden_fixture_e2e`
    // schema gate. It is never reachable from the product: nothing outside
    // `tests/` calls it, and it is `#[doc(hidden)]`. It must seed the episodic,
    // segment, event and profile tiers, which have no guard-routed writer — the
    // archivist and the learning cache reach them the same way, and those two
    // are already allowlisted below/above for the same reason.
    (
        "src/openhuman/memory/store_golden.rs",
        ".profile_conn(",
        "fixture seeder: episodic/segment/event/profile tiers have no guarded writer",
    ),
    (
        "src/openhuman/memory/store_golden.rs",
        "global::client(",
        "resolved only to reach profile_conn() for the fixture seed/read-back",
    ),
    // ── The engine seam ──
    (
        "vendor/tinymemory/core/src/tinycortex/sync.rs",
        "global::client_if_ready(",
        "the TinyCortex engine seam; it sits beneath the contract, not above it",
    ),
];

/// True for source files the lint deliberately does not scan.
///
/// By-path only — see the module docs for why inline `#[cfg(test)]` blocks are
/// left in scope instead of being brace-tracked.
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

/// Every `(repo-relative path, pattern)` pair currently in the production tree.
///
/// Comment lines are skipped so that doc-comment references — of which this
/// module and `memory/global.rs` have several — are not mistaken for calls.
fn scan() -> BTreeSet<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    collect_rs_files(&root.join("src"), &mut files);
    // The memory subsystem was extracted into `tinymemory-core`, and most of
    // the files this lint counts went with it. Scanning only this crate's `src`
    // would quietly drop them from the tally — which would read as "the
    // bypasses were cleaned up" rather than "they moved out of view".
    collect_rs_files(
        &root
            .join("vendor")
            .join("tinymemory")
            .join("core")
            .join("src"),
        &mut files,
    );

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
            for (pattern, _) in BYPASS_PATTERNS {
                if line.contains(pattern) {
                    found.insert((rel.clone(), (*pattern).to_string()));
                }
            }
        }
    }
    found
}

fn allowed_set() -> BTreeSet<(String, String)> {
    ALLOWED
        .iter()
        .map(|(path, pattern, _)| ((*path).to_string(), (*pattern).to_string()))
        .collect()
}

fn render(pairs: impl IntoIterator<Item = (String, String)>) -> String {
    pairs
        .into_iter()
        .map(|(path, pattern)| format!("\n  {path}\t{pattern}"))
        .collect()
}

/// A parser that silently found nothing would turn every other test here into a
/// rubber stamp, so refuse to pass vacuously.
///
/// The literal pinned below is `profile_store()`'s own construction site — a
/// call inside the module that defines the method, so it is the most stable
/// pair available. If the scanner ever stops seeing it, the scanner is broken —
/// fix it, do not relax this assertion.
#[test]
fn bypass_scanner_finds_the_known_bypasses() {
    let found = scan();
    assert!(
        !found.is_empty(),
        "the bypass scanner found nothing at all; every other test in this \
         module would pass vacuously. Fix the scanner, not the assertion."
    );
    let canary = (
        "vendor/tinymemory/core/src/store/client.rs".to_string(),
        ".profile_conn(".to_string(),
    );
    assert!(
        found.contains(&canary),
        "the scanner lost a known bypass ({canary:?}); it is no longer reading \
         the tree correctly. Fix the scanner, not the assertion."
    );
}

/// Every needle in [`BYPASS_PATTERNS`] must still match something.
///
/// Without this a rename turns a pattern into a dead string that quietly stops
/// guarding anything — the same rot [`bypass_allowlist_has_no_stale_entries`]
/// prevents on the allowlist side.
#[test]
fn bypass_patterns_are_all_live() {
    let found = scan();
    let dead: Vec<&str> = BYPASS_PATTERNS
        .iter()
        .filter(|(pattern, _)| !found.iter().any(|(_, p)| p == pattern))
        .map(|(pattern, _)| *pattern)
        .collect();
    assert!(
        dead.is_empty(),
        "these BYPASS_PATTERNS needles match nothing in the tree: {dead:?}\n\
         Either the API was renamed (update the needle) or the last bypass was \
         removed (delete the needle *and* say so in the module docs). A needle \
         that matches nothing is a guard that guards nothing."
    );
}

/// The ratchet, forward direction: no NEW bypass may appear.
#[test]
fn no_new_memory_driver_bypasses() {
    let found = scan();
    let allowed = allowed_set();
    let added: Vec<(String, String)> = found.difference(&allowed).cloned().collect();
    assert!(
        added.is_empty(),
        "new unguarded memory call site(s):{}\n\n\
         Route them through the kernel memory guard \
         (`memory::ops::guard::active_memory_guard`, or `CoreContext::memory()`), \
         or — if the bypass is genuinely required — add them to ALLOWED in this \
         file *with a reason*, and to docs/specs/memory-guard-allowlist.md. \
         The list may shrink; it must never grow.",
        render(added)
    );
}

/// The ratchet, reverse direction — and this half is where the teeth are.
///
/// Fixing a bypass without striking its entry fails the build, so the allowlist
/// cannot silently grow back into the space a cleanup just freed. Mirrors the
/// `stale` computation in `scripts/lib/feature-forwarding.mjs`.
#[test]
fn bypass_allowlist_has_no_stale_entries() {
    let found = scan();
    let allowed = allowed_set();
    let stale: Vec<(String, String)> = allowed.difference(&found).cloned().collect();
    assert!(
        stale.is_empty(),
        "these ALLOWED entries no longer bypass anything:{}\n\n\
         Delete them from this file and from \
         docs/specs/memory-guard-allowlist.md. An allowlist that keeps dead \
         strings stops being evidence of anything.",
        render(stale)
    );
}

/// Cheap guard against the failure mode this tree keeps hitting: a family
/// reorg renames a path and leaves an entry that now allows nothing and hides
/// nothing.
///
/// [`bypass_allowlist_has_no_stale_entries`] would also catch this, but reports
/// it as "no longer bypasses" — which reads like a cleanup to celebrate rather
/// than a rename to finish.
#[test]
fn bypass_allowlist_paths_all_exist() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let missing: Vec<&str> = ALLOWED
        .iter()
        .map(|(path, _, _)| *path)
        .filter(|path| !root.join(path).exists())
        .collect();
    assert!(
        missing.is_empty(),
        "ALLOWED names file(s) that no longer exist: {missing:?}\n\
         A rename left a dead entry. Point it at the new path, or delete it."
    );
}

/// Every allowlist entry must carry a non-empty reason, and must name a needle
/// the lint actually looks for.
///
/// The reason string is the whole value of the list to a future reader; an
/// entry without one is indistinguishable from an oversight.
#[test]
fn bypass_allowlist_entries_are_well_formed() {
    for (path, pattern, reason) in ALLOWED {
        assert!(
            !reason.trim().is_empty(),
            "ALLOWED entry {path} / {pattern} has no reason"
        );
        assert!(
            BYPASS_PATTERNS.iter().any(|(p, _)| p == pattern),
            "ALLOWED entry {path} names {pattern}, which is not in BYPASS_PATTERNS"
        );
    }
}
