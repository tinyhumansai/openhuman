use super::WORKLOAD_ROLES;

#[test]
fn workload_roles_keep_the_public_unprefixed_api_values_and_order() {
    assert_eq!(
        WORKLOAD_ROLES,
        [
            "chat",
            "reasoning",
            "agentic",
            "coding",
            "burst",
            "summarization",
            "vision",
        ]
    );
}

#[test]
fn workload_roles_follow_the_native_workload_route_order() {
    let native = tinyinference_llm::model::workload::WORKLOAD_ROUTE_TIERS
        .iter()
        .map(|route| route.strip_prefix("hint:").expect("native hint route"))
        .collect::<Vec<_>>();
    assert_eq!(native, WORKLOAD_ROLES);
}
