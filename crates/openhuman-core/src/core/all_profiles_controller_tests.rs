use super::*;
use crate::core::runtime::DispatchCtx;

#[test]
fn raw_profiles_operator_registry_matches_its_independent_contract() {
    let contract = super::registry_test_support::BoundFamilyWireContract {
        namespaces: &["profiles"],
        group: DomainGroup::Operator,
        methods: &[
            ("profiles.provision", "openhuman.profiles_provision"),
            ("profiles.deprovision", "openhuman.profiles_deprovision"),
            ("profiles.list", "openhuman.profiles_list"),
            ("profiles.status", "openhuman.profiles_status"),
            (
                "profiles.set_credential",
                "openhuman.profiles_set_credential",
            ),
            (
                "profiles.clear_credential",
                "openhuman.profiles_clear_credential",
            ),
            ("profiles.release", "openhuman.profiles_release"),
        ],
    };
    super::registry_test_support::verify_family_contract(
        registry(),
        internal_registry(),
        &contract,
        &crate::profiles::schemas::profile_parameter_contracts(),
    )
    .expect("raw Profiles entries retain their ordered wire contracts");
}

fn raw_profiles_entries() -> Vec<ControllerEntry> {
    registry()
        .iter()
        .filter(|row| row.controller.schema.namespace == "profiles")
        .map(|row| row.controller.clone())
        .collect()
}

#[test]
fn bound_profile_decoders_keep_permissive_unknown_field_behavior() {
    use serde_json::json;

    let entries = raw_profiles_entries();
    for contract in crate::profiles::schemas::profile_parameter_contracts() {
        let mut params = contract.examples[0]
            .as_object()
            .expect("owner fixture object")
            .clone();
        params.insert("extra_scalar".into(), json!(true));
        params.insert("extra_nested".into(), json!({"ignored":true}));
        entries
            .iter()
            .find(|entry| entry.schema.method_name() == contract.method)
            .unwrap_or_else(|| panic!("missing raw {} entry", contract.method))
            // Deliberately skip schema validation: this pins the legacy
            // DTO/list decoder behavior for keys the schemas do not declare.
            .decode_params(params)
            .unwrap_or_else(|error| {
                panic!(
                    "{} decoder rejected permissive map: {error}",
                    contract.method
                )
            });
    }
}

#[tokio::test]
async fn bound_profile_controller_parses_before_the_saas_guard() {
    let provision = raw_profiles_entries()
        .into_iter()
        .find(|entry| entry.schema.method_name() == "profiles.provision")
        .expect("raw provision entry");
    let parse_error = provision
        .invoke(DispatchCtx::process_default(), Map::new())
        .await
        .expect_err("missing user id fails parsing");
    assert!(parse_error.starts_with("invalid params:"), "{parse_error}");

    let params = serde_json::from_value(serde_json::json!({"user_id":"user-1"}))
        .expect("valid provision params");
    let guard_error = provision
        .invoke(DispatchCtx::process_default(), params)
        .await
        .expect_err("valid params reach the existing SaaS guard");
    assert!(guard_error.contains("SaaS"), "{guard_error}");
}
