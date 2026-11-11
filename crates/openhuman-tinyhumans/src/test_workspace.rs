//! A workspace of this test process's own, for suites that reach TinySecurity.
//!
//! Module-backed checks (privacy, action budget, path floor) resolve the
//! ambient configuration, which otherwise loads from the runner's home
//! (`~/.openhuman/users/local`). Every audited permission then lands on the
//! TinySecurity init audit stream in that one store. nextest runs each test
//! as its own process, so parallel processes claim the same audit sequence,
//! the host's sink refuses the duplicate, and the module fails the check
//! closed for a reason that has nothing to do with the test. A test process
//! also must not write into a developer's real profile.

use std::sync::OnceLock;

/// Point this process's ambient configuration at a workspace of its own,
/// once per process, before the first module call resolves it.
pub(crate) fn isolate() {
    static WORKSPACE: OnceLock<tempfile::TempDir> = OnceLock::new();
    WORKSPACE.get_or_init(|| {
        let dir = tempfile::tempdir().expect("a per-process test workspace");
        log::debug!(
            "[test-workspace] OPENHUMAN_WORKSPACE -> {}",
            dir.path().display()
        );
        std::env::set_var("OPENHUMAN_WORKSPACE", dir.path());
        dir
    });
}
