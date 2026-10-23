use super::*;

/// First link in the chain: `ALL` really does list every variant.
///
/// `DomainGroup::index` is an exhaustive match, so a new variant is a compile
/// error there first; this then fails until it is added to `ALL` and `COUNT` is
/// bumped. Every guard below iterates `ALL`, so they are only as trustworthy as
/// this test.
#[test]
fn domain_group_all_lists_every_variant() {
    assert_eq!(
        DomainGroup::ALL.len(),
        DomainGroup::COUNT,
        "DomainGroup::ALL and DomainGroup::COUNT disagree — a variant was added \
         to one but not the other"
    );
    let mut seen = [false; DomainGroup::COUNT];
    for g in DomainGroup::ALL {
        let i = g.index();
        assert!(
            i < DomainGroup::COUNT,
            "{g:?} has index {i} but COUNT is {} — bump COUNT",
            DomainGroup::COUNT
        );
        assert!(!seen[i], "two variants share index {i}");
        seen[i] = true;
    }
    let missing: Vec<usize> = seen
        .iter()
        .enumerate()
        .filter(|(_, s)| !**s)
        .map(|(i, _)| i)
        .collect();
    assert!(
        missing.is_empty(),
        "DomainGroup::ALL is missing the variant(s) at index {missing:?} — \
         `index()` knows about them but `ALL` does not"
    );
}

/// M5.1 split `memory::all_memory_registered_controllers()` into seven
/// per-family pairs pushed separately in `build_registered_controllers`. This
/// pins the observable result: the `memory` namespace still occupies one
/// contiguous run in the registry, in the aggregator's exact order. A stray
/// push (wrong place, wrong order, a family dropped) fails here.
#[test]
fn memory_controllers_form_one_contiguous_run_in_aggregator_order() {
    let all = all_registered_controllers();
    let positions: Vec<usize> = all
        .iter()
        .enumerate()
        .filter(|(_, c)| c.schema.namespace == "memory")
        .map(|(i, _)| i)
        .collect();

    assert!(!positions.is_empty(), "no memory controllers registered");
    let first = positions[0];
    let expected_run: Vec<usize> = (first..first + positions.len()).collect();
    assert_eq!(
        positions, expected_run,
        "memory controllers are no longer contiguous in the registry"
    );

    let registered: Vec<&'static str> = positions.iter().map(|&i| all[i].schema.function).collect();
    let aggregator: Vec<&'static str> = crate::memory::all_memory_registered_controllers()
        .iter()
        .map(|c| c.schema.function)
        .collect();
    assert_eq!(
        registered, aggregator,
        "registry order for memory.* diverges from the memory schemas aggregator"
    );
}
