use super::*;
use crate::doctor::contract::FindingSeverity;

fn status(state: ModuleState, detail: Option<&str>) -> ModuleStatus {
    ModuleStatus {
        id: "tinydocs".to_owned(),
        description: "fixture module".to_owned(),
        version: "1".to_owned(),
        bus_name: "ai.tinyhumans.tinydocs".to_owned(),
        state,
        source: Some(ModuleSource::Release),
        detail: detail.map(str::to_owned),
    }
}

#[test]
fn failed_module_status_reports_its_load_reason_and_source() {
    let check = ModuleStatusCheck::new(status(ModuleState::Failed, Some("digest mismatch")));

    let finding = check.finding();

    assert_eq!(check.id(), "modules/tinydocs");
    assert_eq!(finding.severity, FindingSeverity::Error);
    assert!(finding.message.contains("digest mismatch"));
    assert!(finding.message.contains("release"));
}

#[test]
fn failed_module_status_does_not_expose_raw_detail_urls_paths_or_credentials() {
    let raw_url =
        "https://module-user:module-password@downloads.invalid/tiny?token=module-query-secret";
    let raw_path = "/Users/private-user/.openhuman/modules/tiny-module";
    let raw_token = "module-error-secret-value";
    let detail = format!("download failed from {raw_url} at {raw_path}; bearer={raw_token}");
    let check = ModuleStatusCheck::new(status(ModuleState::Failed, Some(&detail)));

    let finding = check.finding();

    assert_eq!(finding.severity, FindingSeverity::Error);
    assert!(finding.message.contains("release"));
    assert!(finding.message.contains("failed"));
    for private_value in [
        raw_url,
        raw_path,
        raw_token,
        "module-password",
        "module-query-secret",
    ] {
        assert!(
            !finding.message.contains(private_value),
            "module finding must not expose {private_value:?}: {}",
            finding.message
        );
    }
}

#[test]
fn module_statuses_map_to_their_contract_severities() {
    for state in [ModuleState::Available, ModuleState::Ready] {
        assert_eq!(
            ModuleStatusCheck::new(status(state, None))
                .finding()
                .severity,
            FindingSeverity::Pass
        );
    }
    for state in [ModuleState::Failed, ModuleState::Unsupported] {
        assert_eq!(
            ModuleStatusCheck::new(status(state, None))
                .finding()
                .severity,
            FindingSeverity::Error
        );
    }
    for state in [ModuleState::Disabled, ModuleState::Loading] {
        assert_eq!(
            ModuleStatusCheck::new(status(state, None))
                .finding()
                .severity,
            FindingSeverity::Skipped
        );
    }
}

#[test]
fn module_ids_are_encoded_as_valid_check_id_segments() {
    let check = ModuleStatusCheck::new(ModuleStatus {
        id: "Tiny Module/α".to_owned(),
        ..status(ModuleState::Ready, None)
    });

    assert_eq!(check.id(), "modules/__x_i_54696e79204d6f64756c652fceb1");
}
