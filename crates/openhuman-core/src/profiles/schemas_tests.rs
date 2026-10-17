use super::*;

#[test]
fn every_controller_has_a_schema_and_a_handler() {
    let schemas = all_profiles_controller_schemas();
    let controllers = all_profiles_registered_controllers();
    assert_eq!(schemas.len(), FUNCTIONS.len());
    assert_eq!(controllers.len(), FUNCTIONS.len());
    for (schema, controller) in schemas.iter().zip(&controllers) {
        assert_eq!(schema.namespace, "profiles");
        assert_eq!(schema.function, controller.schema.function);
        assert_ne!(schema.function, "unknown");
    }
}

#[test]
fn the_operator_plane_is_its_own_domain_family() {
    use crate::core::all::DomainGroup;
    use crate::core::runtime::DomainSet;
    assert!(DomainSet::saas().allows(DomainGroup::Operator));
    for preset in [
        DomainSet::full(),
        DomainSet::harness(),
        DomainSet::embedded(),
        DomainSet::kernel(),
        DomainSet::none(),
    ] {
        assert!(
            !preset.allows(DomainGroup::Operator),
            "a single-user preset never serves the operator plane"
        );
    }
}

#[tokio::test]
async fn outside_saas_the_controllers_refuse() {
    let controllers = all_profiles_registered_controllers();
    let list = controllers
        .iter()
        .find(|c| c.schema.function == "list")
        .unwrap();
    let err = (list.handler)(DispatchCtx::process_default(), Default::default())
        .await
        .unwrap_err();
    assert!(err.contains("SaaS"), "{err}");
}
