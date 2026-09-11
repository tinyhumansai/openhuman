use super::*;

#[tokio::test]
async fn current_depth_defaults_to_zero_outside_scope() {
    assert_eq!(current_depth(), 0);
}

#[tokio::test]
async fn scope_sets_and_nests_depth() {
    assert_eq!(current_depth(), 0);
    scope(1, async {
        assert_eq!(current_depth(), 1);
        scope(2, async {
            assert_eq!(current_depth(), 2);
        })
        .await;
        // Restored after the inner scope ends.
        assert_eq!(current_depth(), 1);
    })
    .await;
    assert_eq!(current_depth(), 0);
}

#[test]
fn parse_header_handles_missing_garbage_and_clamps() {
    assert_eq!(parse_header(None), 0);
    assert_eq!(parse_header(Some("")), 0);
    assert_eq!(parse_header(Some("nope")), 0);
    assert_eq!(parse_header(Some("2")), 2);
    // Whitespace-padded, in-range value trims and parses cleanly.
    assert_eq!(parse_header(Some("  3 ")), 3);
    // External input is clamped to the cap — a forged huge value can neither
    // bypass the limit nor overflow a later `depth + 1`.
    assert_eq!(parse_header(Some("999999")), MAX_SUBAGENT_DEPTH);
    assert_eq!(
        parse_header(Some(&usize::MAX.to_string())),
        MAX_SUBAGENT_DEPTH
    );
}
