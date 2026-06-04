use super::*;

fn msg(text: &str, mode: QueueMode) -> QueuedMessage {
    QueuedMessage {
        text: text.to_string(),
        mode,
        client_id: "c1".to_string(),
        thread_id: "t1".to_string(),
        queued_at_ms: 0,
        model_override: None,
        temperature: None,
        profile_id: None,
        locale: None,
    }
}

#[tokio::test]
async fn new_queue_is_empty() {
    let q = RunQueue::new();
    let s = q.status().await;
    assert_eq!(s.total, 0);
    assert_eq!(s.steers, 0);
    assert_eq!(s.followups, 0);
    assert_eq!(s.collects, 0);
}

#[tokio::test]
async fn push_steer_routes_to_steer_lane() {
    let q = RunQueue::new();
    q.push(msg("fix it", QueueMode::Steer)).await;
    let s = q.status().await;
    assert_eq!(s.steers, 1);
    assert_eq!(s.followups, 0);
    assert_eq!(s.collects, 0);
    assert_eq!(s.total, 1);
}

#[tokio::test]
async fn push_followup_routes_to_followup_lane() {
    let q = RunQueue::new();
    q.push(msg("then do this", QueueMode::Followup)).await;
    let s = q.status().await;
    assert_eq!(s.steers, 0);
    assert_eq!(s.followups, 1);
    assert_eq!(s.collects, 0);
}

#[tokio::test]
async fn push_collect_routes_to_collect_lane() {
    let q = RunQueue::new();
    q.push(msg("btw", QueueMode::Collect)).await;
    let s = q.status().await;
    assert_eq!(s.steers, 0);
    assert_eq!(s.followups, 0);
    assert_eq!(s.collects, 1);
}

#[tokio::test]
async fn drain_steers_returns_fifo_and_empties() {
    let q = RunQueue::new();
    q.push(msg("first", QueueMode::Steer)).await;
    q.push(msg("second", QueueMode::Steer)).await;
    let drained = q.drain_steers().await;
    assert_eq!(drained.len(), 2);
    assert_eq!(drained[0].text, "first");
    assert_eq!(drained[1].text, "second");
    assert_eq!(q.status().await.steers, 0);
}

#[tokio::test]
async fn drain_collects_returns_fifo_and_empties() {
    let q = RunQueue::new();
    q.push(msg("a", QueueMode::Collect)).await;
    q.push(msg("b", QueueMode::Collect)).await;
    let drained = q.drain_collects().await;
    assert_eq!(drained.len(), 2);
    assert_eq!(drained[0].text, "a");
    assert_eq!(drained[1].text, "b");
    assert_eq!(q.status().await.collects, 0);
}

#[tokio::test]
async fn drain_followups_returns_fifo_and_empties() {
    let q = RunQueue::new();
    q.push(msg("x", QueueMode::Followup)).await;
    let drained = q.drain_followups().await;
    assert_eq!(drained.len(), 1);
    assert_eq!(drained[0].text, "x");
    assert_eq!(q.status().await.followups, 0);
}

#[tokio::test]
async fn clear_empties_all_lanes() {
    let q = RunQueue::new();
    q.push(msg("s", QueueMode::Steer)).await;
    q.push(msg("f", QueueMode::Followup)).await;
    q.push(msg("c", QueueMode::Collect)).await;
    let dropped = q.clear().await;
    assert_eq!(dropped, 3);
    assert_eq!(q.status().await.total, 0);
}

#[tokio::test]
async fn has_pending_injections_true_for_steers() {
    let q = RunQueue::new();
    assert!(!q.has_pending_injections().await);
    q.push(msg("steer me", QueueMode::Steer)).await;
    assert!(q.has_pending_injections().await);
}

#[tokio::test]
async fn has_pending_injections_true_for_collects() {
    let q = RunQueue::new();
    q.push(msg("ctx", QueueMode::Collect)).await;
    assert!(q.has_pending_injections().await);
}

#[tokio::test]
async fn has_pending_injections_false_for_followups_only() {
    let q = RunQueue::new();
    q.push(msg("later", QueueMode::Followup)).await;
    assert!(!q.has_pending_injections().await);
}

#[tokio::test]
async fn drain_does_not_affect_other_lanes() {
    let q = RunQueue::new();
    q.push(msg("s", QueueMode::Steer)).await;
    q.push(msg("f", QueueMode::Followup)).await;
    q.push(msg("c", QueueMode::Collect)).await;
    let _ = q.drain_steers().await;
    let s = q.status().await;
    assert_eq!(s.steers, 0);
    assert_eq!(s.followups, 1);
    assert_eq!(s.collects, 1);
}

#[tokio::test]
async fn multiple_pushes_accumulate() {
    let q = RunQueue::new();
    for i in 0..5 {
        q.push(msg(&format!("steer-{i}"), QueueMode::Steer)).await;
    }
    for i in 0..3 {
        q.push(msg(&format!("follow-{i}"), QueueMode::Followup))
            .await;
    }
    let s = q.status().await;
    assert_eq!(s.steers, 5);
    assert_eq!(s.followups, 3);
    assert_eq!(s.total, 8);
}

#[tokio::test]
async fn queue_mode_display() {
    assert_eq!(QueueMode::Interrupt.to_string(), "interrupt");
    assert_eq!(QueueMode::Steer.to_string(), "steer");
    assert_eq!(QueueMode::Followup.to_string(), "followup");
    assert_eq!(QueueMode::Collect.to_string(), "collect");
}

#[tokio::test]
async fn queue_mode_default_is_interrupt() {
    assert_eq!(QueueMode::default(), QueueMode::Interrupt);
}
