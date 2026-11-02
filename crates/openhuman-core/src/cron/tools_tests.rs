use super::*;
use crate::config::Config;
use crate::security::SecurityPolicy;
use std::sync::Arc;
use tinytools::ToolExposure;

fn named(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_string()).collect()
}

#[test]
fn a_scope_naming_cron_is_widened_to_the_operation_tools() {
    let mut names = named(&["cron", "resolve_time"]);
    widen_named_scope(&mut names);
    let mut expected = named(&["cron", "resolve_time"]);
    expected.extend(named(CRON_OPERATION_TOOLS));
    assert_eq!(names, expected);
}

#[test]
fn a_scope_without_cron_is_left_alone() {
    let mut names = named(&["cron_list", "resolve_time"]);
    widen_named_scope(&mut names);
    assert_eq!(names, named(&["cron_list", "resolve_time"]));
}

#[test]
fn widening_never_names_a_tool_twice() {
    let mut names = named(&["cron_remove", "cron"]);
    widen_named_scope(&mut names);
    for tool in CRON_OPERATION_TOOLS {
        assert_eq!(
            names.iter().filter(|name| name == tool).count(),
            1,
            "{tool}"
        );
    }
}

#[test]
fn the_operation_tools_are_advertised_and_the_collapsed_cron_is_deferred() {
    let tools = tools(
        Arc::new(Config::default()),
        &Arc::new(SecurityPolicy::default()),
    );
    for tool in &tools {
        let expected = if tool.name() == CRON_TOOL_NAME {
            ToolExposure::Deferred
        } else {
            ToolExposure::Direct
        };
        assert_eq!(tool.exposure(), expected, "{}", tool.name());
    }
    let mut operations: Vec<&str> = tools
        .iter()
        .map(|tool| tool.name())
        .filter(|name| *name != CRON_TOOL_NAME)
        .collect();
    operations.sort_unstable();
    let mut expected = CRON_OPERATION_TOOLS.to_vec();
    expected.sort_unstable();
    assert_eq!(operations, expected);
}

#[test]
fn every_operation_tool_is_described_in_one_short_line() {
    let tools = tools(
        Arc::new(Config::default()),
        &Arc::new(SecurityPolicy::default()),
    );
    for tool in tools.iter().filter(|tool| tool.name() != CRON_TOOL_NAME) {
        let description = tool.description();
        assert!(
            description.len() <= 120 && !description.contains('\n'),
            "`{}` description is {} chars: the rules belong in the scheduling guide",
            tool.name(),
            description.len()
        );
    }
}

#[test]
fn the_operation_schemas_stay_within_their_budget() {
    // What every call carries once the pack is advertised. The collapsed
    // `cron` cost ~830 chars of description and ~2.8k of schema on every
    // call; together the six stay well under that.
    let tools = tools(
        Arc::new(Config::default()),
        &Arc::new(SecurityPolicy::default()),
    );
    let total: usize = tools
        .iter()
        .filter(|tool| tool.name() != CRON_TOOL_NAME)
        .map(|tool| tool.description().len() + tool.parameters_schema().to_string().len())
        .sum();
    assert!(total <= 3_200, "the scheduling pack costs {total} chars");
}
