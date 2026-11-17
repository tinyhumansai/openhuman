mod common;
#[path = "support/attached_tools.rs"]
mod scenarios;

#[test]
fn attached_tools_survive_clones_and_session_resume() {
    scenarios::attached_tools_survive_clones_and_session_resume();
}
