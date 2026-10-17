//! The turn gate queues one profile's turns and leaves other profiles alone.

use std::time::Duration;

use super::*;

#[tokio::test]
async fn a_profile_runs_one_turn_at_a_time() {
    let first = acquire_profile("gate-a").await;
    let second = tokio::spawn(acquire_profile("gate-a"));
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!second.is_finished(), "the second turn waits for the first");
    drop(first);
    tokio::time::timeout(Duration::from_secs(5), second)
        .await
        .expect("released gate admits the next turn")
        .unwrap();
}

#[tokio::test]
async fn other_profiles_are_not_held_up() {
    let _held = acquire_profile("gate-b").await;
    tokio::time::timeout(Duration::from_secs(5), acquire_profile("gate-c"))
        .await
        .expect("a different profile's gate is free");
}

#[tokio::test]
async fn work_without_a_profile_is_not_gated() {
    assert!(acquire(&crate::core::runtime::DispatchCtx::ambient())
        .await
        .is_none());
}
