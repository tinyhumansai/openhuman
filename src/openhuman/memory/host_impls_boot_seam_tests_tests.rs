use super::*;

/// Installing the seams must satisfy the engine's `require_embedding_host`.
///
/// This began as the regression for an outage #5560 shipped and had to take
/// back: the seam install was removed from all three boot sites on the
/// reasoning that "this process embeds no engine, so there is nothing to call
/// back". At the time it did embed one — `session::builder::factory` reached
/// `store::factories::create_session_memory_with_local_ai`, which calls
/// `require_embedding_host()` — and every chat turn died with
///
///   no EmbeddingHost installed — the host must call
///   memory::embedding_host::set_embedding_host during startup wiring
///
/// **That caller is gone, and so is the production install.** `tinymemory-core`
/// is out of the product build now — a `[dev-dependencies]` entry plus an
/// `optional` normal one that only `memory-engine-seams` and `rss-bench` turn
/// on — and [`super`] is gated on the same feature. What the assertion guards
/// has therefore changed, and it is worth being exact about which claim is
/// still being tested: not that *boot* wires the engine, but that
/// [`install_for_tests`] does — the ~90 test call sites that stand up an
/// in-process engine all depend on it, and nothing in a build or a type check
/// says whether the `set_*` globals actually took.
///
/// The original reason for asserting the engine's own accessor rather than a
/// local flag is unchanged, and is why this survived the migration rather than
/// being deleted with the boot sites: it keeps testing the thing the engine
/// actually reads.
///
/// The production half of the old claim now lives in
/// `runtime::context::init_stores`, which still installs the *contract* event
/// sink — a `tinymemory-api` seam with a live host-side publisher in
/// `memory::sync::composio::bus`, and one that drops silently rather than
/// loudly when unwired.
#[test]
fn installing_the_seams_satisfies_the_engines_embedding_host() {
    install_for_tests();

    assert!(
        tinymemory_core::embedding_host::embedding_host().is_some(),
        "install_for_tests installed no EmbeddingHost; every test that stands up an \
         in-process memory client calls require_embedding_host() and will fail"
    );
    assert!(
        tinymemory_core::embedding_host::require_embedding_host().is_ok(),
        "require_embedding_host must succeed once the test seams are in"
    );
}
