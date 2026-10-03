use super::load_declared_modules;
use crate::config::Config;

#[tokio::test]
async fn boot_is_a_no_op_when_modules_are_disabled() {
    // Must not start a broker as a side effect of being switched off.
    let mut config = Config::default();
    config.modules.enabled = false;
    load_declared_modules(&config).await;
}

#[tokio::test]
async fn boot_tolerates_an_empty_search_path() {
    // The ordinary case on a fresh machine: nothing installed, nothing eager,
    // and boot must complete rather than warn or fail.
    let mut config = Config::default();
    config.modules.enabled = true;
    config.modules.allow_download = false;
    load_declared_modules(&config).await;
}
