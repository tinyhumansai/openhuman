use super::*;

#[test]
fn empty_stdout_is_a_noop() {
    let output = parse_stdout("   \n").expect("empty stdout is allowed");
    assert!(output.permission.is_none());
}

#[test]
fn plain_json_is_parsed() {
    let output = parse_stdout(r#"{"permission":"deny","agent_message":"no"}"#).unwrap();
    assert!(output.is_deny());
    assert_eq!(output.agent_message.as_deref(), Some("no"));
}

#[test]
fn json_after_log_lines_is_recovered() {
    let stdout = "checking policy...\nstill checking\n{\"permission\":\"allow\"}\n";
    let output = parse_stdout(stdout).unwrap();
    assert_eq!(output.permission, Some(HookPermission::Allow));
}

#[test]
fn unparseable_stdout_is_an_error() {
    let error = parse_stdout("not json at all").unwrap_err();
    assert!(error.contains("not a hook decision object"), "{error}");
}

#[test]
fn truncate_marks_elision() {
    assert_eq!(truncate("abcdef", 3), "abc…");
    assert_eq!(truncate("abc", 3), "abc");
}
