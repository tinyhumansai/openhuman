use super::*;
use crate::modules::types::ModuleState;

fn config_without(component: &str) -> Config {
    let mut config = Config::default();
    config.modules.enabled = true;
    config.modules.allow_download = true;
    assert!(config.components.set(component, false));
    config
}

#[tokio::test]
async fn a_disabled_component_never_reaches_download_or_dlopen() {
    let config = config_without("mcp");
    let err = ensure_loaded_within(&config, "tinymcp", None)
        .await
        .unwrap_err();
    assert_eq!(err, LoadError::Disabled("mcp"));
    assert_eq!(load_error_message(err), "COMPONENT_DISABLED:mcp");
}

#[test]
fn list_reports_disabled_for_a_switched_off_component() {
    let config = config_without("connectors");
    let statuses = list(&config);
    let status = statuses.iter().find(|s| s.id == "tinyconnectors").unwrap();
    assert_eq!(status.state, ModuleState::Disabled);
    assert!(status.source.is_none());
    assert!(status.detail.as_deref().unwrap().contains("[components]"));
    let other = statuses.iter().find(|s| s.id == "tinymcp").unwrap();
    assert_ne!(other.state, ModuleState::Disabled);
}

#[test]
fn disabled_lib_stems_cover_only_disabled_modules() {
    let config = config_without("search");
    let stems = disabled_lib_stems(&config);
    let search = registry::find("tinysearch").unwrap().source.lib_name;
    assert!(stems.contains(&search.to_string()));
    assert!(stems.contains(&format!("lib{search}")));
    let mcp = registry::find("tinymcp").unwrap().source.lib_name;
    assert!(!stems.contains(&mcp.to_string()));
    assert!(disabled_lib_stems(&Config::default()).is_empty());
}

#[cfg(feature = "web3")]
#[tokio::test]
async fn a_disabled_wallet_never_loads_the_tinywallet_module() {
    let config = config_without("wallet");
    let err = ensure_loaded_within(&config, "tinywallet", None)
        .await
        .unwrap_err();
    assert_eq!(err, LoadError::Disabled("wallet"));
    assert_eq!(load_error_message(err), "COMPONENT_DISABLED:wallet");
}

#[cfg(feature = "hosting")]
#[tokio::test]
async fn a_disabled_hosting_component_never_loads_the_tinyhosts_module() {
    let config = config_without("hosting");
    let err = ensure_loaded_within(&config, "tinyhosts", None)
        .await
        .unwrap_err();
    assert_eq!(err, LoadError::Disabled("hosting"));
    assert_eq!(load_error_message(err), "COMPONENT_DISABLED:hosting");
    let statuses = list(&config);
    let status = statuses.iter().find(|s| s.id == "tinyhosts").unwrap();
    assert_eq!(status.state, ModuleState::Disabled);
}

#[tokio::test]
async fn juice_off_refuses_tinyjuice_before_download_or_load() {
    let config = config_without("juice");
    let err = ensure_loaded_within(&config, "tinyjuice", None)
        .await
        .unwrap_err();
    assert_eq!(err, LoadError::Disabled("juice"));
    // The resolution slot is process-global, and other tests legitimately
    // load tinyjuice into it while this one runs, so `state_of("tinyjuice")`
    // cannot be asserted here. The refusal itself is the proof: a claim on
    // the slot would have answered with a download or load outcome, never
    // `Disabled`, and the listing below reports `Disabled` with no source.
    let statuses = list(&config);
    let status = statuses.iter().find(|s| s.id == "tinyjuice").unwrap();
    assert_eq!(status.state, ModuleState::Disabled);
    assert!(status.source.is_none());
}

#[cfg(feature = "voice")]
#[tokio::test]
async fn voice_off_refuses_tinyvoice_before_download_or_load() {
    let config = config_without("voice");
    let err = ensure_loaded_within(&config, "tinyvoice", None)
        .await
        .unwrap_err();
    assert_eq!(err, LoadError::Disabled("voice"));
    assert_eq!(load_error_message(err), "COMPONENT_DISABLED:voice");
    let statuses = list(&config);
    let status = statuses.iter().find(|s| s.id == "tinyvoice").unwrap();
    assert_eq!(status.state, ModuleState::Disabled);
    assert!(status.source.is_none());
}

#[tokio::test]
async fn a_disabled_sandbox_never_loads_the_tinybox_module() {
    let config = config_without("sandbox");
    let err = ensure_loaded_within(&config, "tinybox", None)
        .await
        .unwrap_err();
    assert_eq!(err, LoadError::Disabled("sandbox"));
    assert_eq!(load_error_message(err), "COMPONENT_DISABLED:sandbox");
    let statuses = list(&config);
    let status = statuses.iter().find(|s| s.id == "tinybox").unwrap();
    assert_eq!(status.state, ModuleState::Disabled);
}
