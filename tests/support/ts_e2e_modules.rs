//! TS-E2E X6: the modules `scripts/ci/ts-e2e-stage-modules.sh` staged as an
//! installer bundle, and the environment a fixture needs to load them the way
//! the shipped app does (bundled release, no local-file module path).

/// The ids the staging script found loadable on this runner.
pub fn staged() -> Vec<String> {
    std::env::var("OPENHUMAN_TS_E2E_MODULES")
        .unwrap_or_default()
        .split(',')
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Fail with the reason when `id` was not staged, instead of a later load error.
pub fn require_staged(id: &str) {
    let staged = staged();
    assert!(
        staged.iter().any(|staged| staged == id),
        "TS-E2E: module {id} is not staged on this runner (staged: {staged:?}); \
         run under `source scripts/ci/ts-e2e-stage-modules.sh`"
    );
}

/// Load modules from the staged bundle only: a local-file module path would
/// bypass the release source that callback-bound modules require.
pub fn use_bundled_modules() {
    assert!(
        std::env::var_os("OPENHUMAN_BUNDLED_MODULES").is_some(),
        "TS-E2E: OPENHUMAN_BUNDLED_MODULES is unset; source scripts/ci/ts-e2e-stage-modules.sh"
    );
    std::env::remove_var("OPENHUMAN_MODULE_PATH");
}
