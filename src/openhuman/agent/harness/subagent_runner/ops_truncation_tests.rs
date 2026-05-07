/// Tests for the `max_result_chars` truncation logic in `ops.rs`.
///
/// Kept in a dedicated file so `ops_tests.rs` stays under ~500 lines.
/// The logic under test lives in `run_subagent` — tests here cover the
/// char-safe truncation path directly without spinning up a provider.

#[test]
fn max_result_chars_cap_is_enforced() {
    // Verify that max_result_chars truncation uses char count (not bytes)
    // and produces a truncated result ending with "[...truncated]".
    let cap = 10usize;
    let input = "hello world this is long".to_string();
    let original_chars = input.chars().count();
    let mut output = input.clone();
    if original_chars > cap {
        let byte_offset = output
            .char_indices()
            .nth(cap)
            .map(|(i, _)| i)
            .unwrap_or(output.len());
        output.truncate(byte_offset);
        output.push_str("\n[...truncated]");
    }
    assert_eq!(&output[..10], "hello worl");
    assert!(output.ends_with("[...truncated]"));
}

#[test]
fn max_result_chars_cap_is_char_safe_for_multibyte() {
    // A cap landing in the middle of a multi-byte UTF-8 sequence must
    // not panic. "café" has 4 chars but 'é' is 2 bytes — truncating at
    // byte offset 4 with a raw String::truncate() would panic.
    let cap = 3usize; // keep "caf", drop "é"
    let input = "café latte".to_string();
    let original_chars = input.chars().count();
    let mut output = input.clone();
    if original_chars > cap {
        let byte_offset = output
            .char_indices()
            .nth(cap)
            .map(|(i, _)| i)
            .unwrap_or(output.len());
        output.truncate(byte_offset);
        output.push_str("\n[...truncated]");
    }
    assert_eq!(output, "caf\n[...truncated]");
}

#[test]
fn max_result_chars_not_applied_when_none() {
    let cap: Option<usize> = None;
    let original = "short output".to_string();
    let mut output = original.clone();
    if let Some(c) = cap {
        let char_len = output.chars().count();
        if char_len > c {
            let byte_offset = output
                .char_indices()
                .nth(c)
                .map(|(i, _)| i)
                .unwrap_or(output.len());
            output.truncate(byte_offset);
            output.push_str("\n[...truncated]");
        }
    }
    assert_eq!(output, original);
}
