mod common;
#[path = "support/host_only_tools.rs"]
mod scenarios;

#[test]
fn host_only_forces_read_only_access_and_sandbox() {
    scenarios::host_only_forces_read_only_access_and_sandbox();
}
