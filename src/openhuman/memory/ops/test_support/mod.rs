//! Shared test infrastructure for `memory::ops` submodule tests.
//!
//! All `ops` submodules that need one workspace call
//! [`shared_memory_test_workspace`] instead of creating their own
//! `OnceLock<PathBuf>`. Sharing one leaked workspace is what makes concurrent
//! tests agree on a path rather than racing to bind different ones.
//!
//! # It no longer boots an engine
//!
//! It used to also expose `ensure_shared_memory_client`, which called
//! `tinymemory_core::global::init` to hand `ops` tests a real store to write
//! rows into and read back. That is gone with the engine (openhuman#6161); what
//! survives is the part that was never engine work — one agreed-upon temp
//! directory.

use std::path::PathBuf;
use std::sync::OnceLock;

/// Return the process-global workspace used by memory tests without starting a
/// client.
///
/// Binding tests use this narrower helper because constructing a module-backed
/// provider is intentionally synchronous and lazy. The live client starts a
/// Tokio ingestion worker, so initializing it here would make a mere bind
/// depend on whichever test happened to install a reactor first.
pub(crate) fn shared_memory_test_workspace() -> PathBuf {
    static WORKSPACE: OnceLock<PathBuf> = OnceLock::new();
    WORKSPACE
        .get_or_init(|| {
            let tmp = tempfile::TempDir::new().expect("tempdir");
            let path = tmp.path().join("workspace");
            std::fs::create_dir_all(&path).expect("workspace dir");
            std::mem::forget(tmp);

            // Bind a driver over it, which is the half callers actually depend
            // on, and bind it exactly once.
            //
            // This helper replaced `ensure_shared_memory_client`, which booted
            // the in-process engine and bound *that*. Handing back only the
            // directory was the wrong half of the trade: `memory::ops`
            // handlers resolve through the bound driver, so with nothing bound
            // they fall through to the module path and fail with "the memory
            // module failed to load" — an error about a missing artifact, in a
            // unit test that never wanted one.
            //
            // Binding must happen **inside** `get_or_init`. `install_for_test`
            // *replaces* the cached binding rather than leaving an existing one
            // alone (`BINDINGS.write().insert(key, ..)`), and the driver it
            // installs stores rows in itself. Calling it per caller therefore
            // hands every test a brand-new empty store, and — because most of
            // these tests do not hold `GLOBAL_MEMORY_TEST_LOCK` — a second
            // test's setup wipes the rows a first test has already written and
            // is about to read back. That failed 16 handler tests as
            // "the write is not visible", which reads like a driver that
            // discards writes and is really a fixture that discards stores.
            let mut config = crate::openhuman::config::Config::default();
            config.workspace_dir = path.clone();
            crate::openhuman::memory::test_support::install_memory_driver_for_test(&config);
            path
        })
        .clone()
}
