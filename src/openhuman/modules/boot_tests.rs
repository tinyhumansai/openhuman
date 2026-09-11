use super::{load_declared_modules, should_eager_load};
use crate::openhuman::config::Config;

#[test]
fn tinymemory_is_not_eager_when_memory_is_disabled() {
    let mut config = Config::default();
    config.subsystems.memory.driver = "null".to_string();
    let record = super::registry::find(super::super::memory::MODULE_ID)
        .expect("tinymemory is a registered module");
    assert!(!should_eager_load(record, &config));
}

#[test]
fn tinymemory_is_eager_for_the_default_module_driver() {
    let mut config = Config::default();
    // The legacy persisted id aliases to the TinyMemory module until the
    // shared API changes its default string.
    config.subsystems.memory.driver = "tinycortex".to_string();
    let record = super::registry::find(super::super::memory::MODULE_ID)
        .expect("tinymemory is a registered module");
    assert!(should_eager_load(record, &config));
}

#[test]
fn other_eager_records_are_unconditional() {
    // Non-memory eager records (today, none — but the rule must not
    // silently start gating an unrelated module the moment one is added
    // and marked `Eager`) are unaffected by the memory driver selection.
    let config = Config::default();
    for record in super::registry::ALL {
        if record.id == super::super::memory::MODULE_ID {
            continue;
        }
        assert!(
            should_eager_load(record, &config),
            "record '{}' should be unconditionally eager-eligible",
            record.id
        );
    }
}

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
