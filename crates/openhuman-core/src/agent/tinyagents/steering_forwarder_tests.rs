use super::*;
use tinyagents_harness::run_queue::QueueLane;

#[test]
fn recovered_steer_becomes_a_queued_turn_on_the_forwarders_thread() {
    let turn = QueuedTurn::requeued("id-1".into(), "text".into(), "thread-9", 42);
    assert_eq!(turn.id(), "id-1");
    assert_eq!(turn.text(), "text");
    assert_eq!(turn.thread_id, "thread-9");
    assert_eq!(turn.queued_at_ms, 42);
    assert!(turn.client_id.is_empty());
    assert!(turn.model_override.is_none() && turn.temperature.is_none() && turn.locale.is_none());
}

#[tokio::test]
async fn the_feed_drains_both_lanes_into_the_steering_handle() {
    let queue = Arc::new(RunQueue::<QueuedTurn>::new());
    queue
        .push(
            QueueLane::Steer,
            QueuedTurn::requeued("s".into(), "go left".into(), "t", 1),
        )
        .await;
    queue
        .push(
            QueueLane::Collect,
            QueuedTurn::requeued("c".into(), "fyi".into(), "t", 1),
        )
        .await;
    let handle = SteeringHandle::allow_all();
    queue_feed(queue.clone()).prime(&handle, "t").await;
    assert_eq!(handle.drain().len(), 2);
    assert_eq!(queue.status().await.total, 0);
}

#[test]
fn the_registrar_publishes_and_removes_the_subagent_handle() {
    let task_id = TaskId::new("registrar-task");
    let handle = SteeringHandle::allow_all();
    SharedSteeringRegistrar.register(task_id.as_str(), &handle);
    assert!(shared_steering_registry().get(&task_id).is_some());
    SharedSteeringRegistrar.deregister(task_id.as_str());
    assert!(
        shared_steering_registry().get(&task_id).is_none(),
        "deregister must remove the sub-agent handle"
    );
}
