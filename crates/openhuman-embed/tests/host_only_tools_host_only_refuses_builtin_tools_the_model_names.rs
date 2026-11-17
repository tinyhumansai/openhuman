mod common;
#[path = "support/host_only_tools.rs"]
mod scenarios;

#[test]
fn host_only_refuses_builtin_tools_the_model_names() {
    scenarios::host_only_refuses_builtin_tools_the_model_names();
}
