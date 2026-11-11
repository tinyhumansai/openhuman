mod common;
#[path = "support/host_only_tools.rs"]
mod scenarios;

#[test]
fn host_only_advertises_exactly_the_host_tools_under_a_bare_prompt() {
    scenarios::host_only_advertises_exactly_the_host_tools_under_a_bare_prompt();
}
