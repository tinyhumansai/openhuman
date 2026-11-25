use super::*;

pub(super) struct BoundFamilyWireContract<'a> {
    pub namespaces: &'a [&'a str],
    pub group: DomainGroup,
    pub methods: &'a [(&'a str, &'a str)],
}

pub(super) fn verify_family_contract(
    public: &[GroupedController],
    internal: &[GroupedController],
    contract: &BoundFamilyWireContract<'_>,
    fixtures: &[ControllerParamContract],
) -> Result<(), String> {
    use std::collections::BTreeSet;

    let namespaces: BTreeSet<_> = contract.namespaces.iter().copied().collect();
    if namespaces.is_empty() || namespaces.len() != contract.namespaces.len() {
        return Err("empty or duplicate namespace contract".into());
    }
    let rows: Vec<_> = public
        .iter()
        .filter(|row| namespaces.contains(row.controller.schema.namespace))
        .collect();
    if internal
        .iter()
        .any(|row| namespaces.contains(row.controller.schema.namespace))
    {
        return Err(format!(
            "{:?} leaked into the internal registry",
            contract.namespaces
        ));
    }
    if rows.is_empty() {
        return Err(format!("no public rows for {:?}", contract.namespaces));
    }
    if rows.iter().any(|row| row.group != contract.group) {
        return Err(format!(
            "{:?} registered under the wrong domain group",
            contract.namespaces
        ));
    }
    let actual_methods: Vec<_> = rows
        .iter()
        .map(|row| row.controller.schema.method_name())
        .collect();
    let expected_methods: Vec<_> = contract.methods.iter().map(|(method, _)| *method).collect();
    if actual_methods
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        != expected_methods
    {
        return Err(format!(
            "{:?} method inventory/order mismatch: expected {expected_methods:?}, got {actual_methods:?}",
            contract.namespaces
        ));
    }
    let actual_rpc: Vec<_> = rows
        .iter()
        .map(|row| row.controller.rpc_method_name())
        .collect();
    let expected_rpc: Vec<_> = contract.methods.iter().map(|(_, rpc)| *rpc).collect();
    if actual_rpc.iter().map(String::as_str).collect::<Vec<_>>() != expected_rpc {
        return Err(format!(
            "{:?} RPC inventory/order mismatch: expected {expected_rpc:?}, got {actual_rpc:?}",
            contract.namespaces
        ));
    }
    let fixture_methods: Vec<_> = fixtures.iter().map(|fixture| fixture.method).collect();
    if fixture_methods != expected_methods {
        return Err(format!(
            "{:?} fixture inventory/order mismatch: expected {expected_methods:?}, got {fixture_methods:?}",
            contract.namespaces
        ));
    }
    let entries: Vec<_> = rows.into_iter().map(|row| row.controller.clone()).collect();
    crate::core::all::verify_parameter_contracts(&entries, fixtures)
}
