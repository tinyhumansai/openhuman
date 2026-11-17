use super::*;

#[test]
fn javascript_harness_reads_input_and_serializes_return_value() {
    let script = js_harness("return input[0];");
    assert!(script.contains("JSON.parse"));
    assert!(script.contains("return input[0];"));
    assert!(script.contains("JSON.stringify"));
}

#[test]
fn empty_python_source_uses_a_valid_pass_body() {
    let script = python_harness("   ");
    assert!(script.contains("def __user_fn__(input):\n    pass\n    return None"));
}

#[test]
fn shell_quote_escapes_embedded_single_quotes() {
    assert_eq!(shell_quote("a'b"), "'a'\\''b'");
}
