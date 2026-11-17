use super::*;

#[test]
fn permanent_tools_survive_packed_and_deferred_filters_but_revocation_wins() {
    let mut visible = [
        "packed_permanent".to_string(),
        "deferred_permanent".to_string(),
        "ordinary".to_string(),
        "revoked_permanent".to_string(),
    ]
    .into_iter()
    .collect::<std::collections::HashSet<_>>();
    let mut deferred = std::collections::HashSet::new();
    let mut permanent = [
        "packed_permanent".to_string(),
        "deferred_permanent".to_string(),
        "revoked_permanent".to_string(),
    ]
    .into_iter()
    .collect::<std::collections::HashSet<_>>();
    let previous = ["revoked_permanent".to_string()]
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    let next = std::collections::HashSet::new();
    let discovery_deferred = ["deferred_permanent".to_string(), "ordinary".to_string()]
        .into_iter()
        .collect();

    reconcile_surface_names(
        &mut visible,
        &mut deferred,
        &mut permanent,
        &previous,
        &next,
        false,
        Some(discovery_deferred),
        |visible| {
            visible.remove("packed_permanent");
        },
    );

    assert!(visible.contains("packed_permanent"));
    assert!(visible.contains("deferred_permanent"));
    assert!(!visible.contains("ordinary"));
    assert!(!visible.contains("revoked_permanent"));
    assert!(!deferred.contains("deferred_permanent"));
    assert!(deferred.contains("ordinary"));
    assert!(!permanent.contains("revoked_permanent"));
}

#[test]
fn packed_deferred_tool_keeps_its_use_skill_route_visible() {
    let packed_deferred = "packed_deferred_tool".to_string();
    for (previous_names, discovery_deferred) in [
        (std::collections::HashSet::new(), true),
        ([packed_deferred.clone()].into_iter().collect(), false),
    ] {
        let mut visible = std::collections::HashSet::new();
        let mut deferred = std::collections::HashSet::new();
        let mut permanent = std::collections::HashSet::new();
        let next = [packed_deferred.clone()].into_iter().collect();
        let discovery_deferred = discovery_deferred.then(|| {
            [packed_deferred.clone()]
                .into_iter()
                .collect::<std::collections::HashSet<_>>()
        });
        let expect_deferred = discovery_deferred.is_some();

        reconcile_surface_names(
            &mut visible,
            &mut deferred,
            &mut permanent,
            &previous_names,
            &next,
            true,
            discovery_deferred,
            |visible| {
                if visible.remove(&packed_deferred) {
                    visible.insert("use_skill".to_string());
                }
            },
        );

        assert!(visible.contains("use_skill"));
        assert!(!visible.contains(&packed_deferred));
        assert_eq!(deferred.contains(&packed_deferred), expect_deferred);
    }
}

#[test]
fn explicit_belt_does_not_reopen_an_existing_synthesized_tool() {
    let synthesized_name = "restricted_delegate".to_string();
    let mut visible = std::collections::HashSet::new();
    let mut deferred = std::collections::HashSet::new();
    let mut permanent = std::collections::HashSet::new();
    let previous = [synthesized_name.clone()].into_iter().collect();
    let next = [synthesized_name.clone()].into_iter().collect();

    reconcile_surface_names(
        &mut visible,
        &mut deferred,
        &mut permanent,
        &previous,
        &next,
        false,
        None,
        |_| {},
    );

    assert!(!visible.contains(&synthesized_name));
}
