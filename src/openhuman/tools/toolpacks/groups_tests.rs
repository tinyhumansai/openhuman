use super::*;

#[test]
fn the_default_is_todays_behaviour() {
    // Every compiled-in group withheld — what the pack table meant before
    // this type existed. A host that never calls `tool_groups` must be
    // unaffected, so this is the assertion that pins "no behaviour change".
    let g = ToolGroups::default();
    for id in ToolGroups::ids() {
        assert_eq!(g.mode(id), GroupMode::Withheld, "group `{id}` drifted");
    }
}

#[test]
fn presets_are_uniform() {
    for id in ToolGroups::ids() {
        assert_eq!(ToolGroups::advertised().mode(id), GroupMode::Advertised);
        assert_eq!(ToolGroups::none().mode(id), GroupMode::Off);
        assert_eq!(ToolGroups::packed().mode(id), GroupMode::Withheld);
    }
}

#[test]
fn with_sets_one_group_and_leaves_the_rest() {
    let g = ToolGroups::none().with("documents", GroupMode::Advertised);
    assert_eq!(g.mode("documents"), GroupMode::Advertised);
    assert_eq!(g.mode("crypto"), GroupMode::Off);
}

#[test]
fn an_unknown_group_id_is_ignored_not_fatal() {
    // A group id is data. A build that compiled a family out should not
    // panic a host whose config still names it.
    let g = ToolGroups::none().with("no-such-group", GroupMode::Advertised);
    for id in ToolGroups::ids() {
        assert_eq!(g.mode(id), GroupMode::Off);
    }
}

#[test]
fn a_tool_in_no_group_is_never_withheld() {
    // `mode_for_tool` is consulted for every tool in the belt, most of
    // which belong to no pack. Reporting anything but `Advertised` there
    // would withhold the baseline surface.
    assert_eq!(
        ToolGroups::none().mode_for_tool("apply_patch"),
        GroupMode::Advertised
    );
    assert_eq!(
        ToolGroups::none().mode_for_tool("shell"),
        GroupMode::Advertised
    );
}

#[test]
fn mode_for_tool_follows_its_pack() {
    let g = ToolGroups::default().with("system", GroupMode::Off);
    assert_eq!(g.mode_for_tool("doctor_health"), GroupMode::Off);
    // A different pack is untouched.
    assert_eq!(g.mode_for_tool("wallet_status"), GroupMode::Withheld);
}

#[test]
fn every_group_id_is_reachable_by_name() {
    // `ids()` is what an embedder enumerates; `index_of` is what `with`
    // resolves. A pack id that round-trips through neither would be
    // unselectable from the library surface.
    for id in ToolGroups::ids() {
        assert!(ToolGroups::index_of(id).is_some(), "`{id}` is unselectable");
    }
    assert_eq!(ToolGroups::ids().count(), GROUP_COUNT);
}
