use super::super::super::subagent_depth::{current_depth, scope, MAX_SUBAGENT_DEPTH};

#[tokio::test]
async fn child_depth_is_bounded_per_chain() {
    // The dispatch refuses when `current_depth() >= MAX` (guarding before the
    // `+1` so a clamped depth at the cap can't overflow). At depth MAX the
    // next subagent is refused; below it, allowed. (Parallel unrelated chains
    // each start at 0 — no interference.)
    assert_eq!(current_depth(), 0, "top level starts at depth 0");
    scope(MAX_SUBAGENT_DEPTH, async {
        assert!(
            current_depth() >= MAX_SUBAGENT_DEPTH,
            "at the cap, spawning a deeper child must be refused"
        );
    })
    .await;
    scope(MAX_SUBAGENT_DEPTH - 1, async {
        assert!(
            current_depth() < MAX_SUBAGENT_DEPTH,
            "one below the cap, a child is still allowed"
        );
    })
    .await;
}
