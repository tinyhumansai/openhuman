use super::*;

#[tokio::test]
async fn subagent_spawned_forwards_parent_call_id() {
    let mut events = super::super::super::event_bus::subscribe_web_channel_events();
    let thread_id = "thread-c1-spawned";
    let tx = spawn_test_bridge(thread_id, "req-c1-spawned");

    tx.send(AgentProgress::SubagentSpawned {
        agent_id: "researcher".into(),
        task_id: "sub-c1".into(),
        mode: "typed".into(),
        dedicated_thread: false,
        prompt_chars: 4,
        prompt: "help".into(),
        worker_thread_id: None,
        display_name: None,
        parent_call_id: Some("call-parent-1".into()),
    })
    .await
    .unwrap();

    let ev = recv_for_thread(&mut events, thread_id).await;
    assert_eq!(ev.event, "subagent_spawned");
    let subagent = ev.subagent.expect("subagent detail present");
    assert_eq!(subagent.parent_call_id.as_deref(), Some("call-parent-1"));
}

/// Terminal sub-agent events (`_completed`/`_failed`/`_awaiting_user`) must
/// keep carrying the same `parent_call_id` the spawn recorded, even though
/// those `AgentProgress` variants don't repeat it — the bridge remembers it
/// per `task_id` (#C1).
#[tokio::test]
async fn subagent_completed_carries_parent_call_id_and_capped_output() {
    let mut events = super::super::super::event_bus::subscribe_web_channel_events();
    let thread_id = "thread-c1-completed";
    let tx = spawn_test_bridge(thread_id, "req-c1-completed");

    tx.send(AgentProgress::SubagentSpawned {
        agent_id: "researcher".into(),
        task_id: "sub-c1-done".into(),
        mode: "typed".into(),
        dedicated_thread: false,
        prompt_chars: 4,
        prompt: "help".into(),
        worker_thread_id: None,
        display_name: None,
        parent_call_id: Some("call-parent-2".into()),
    })
    .await
    .unwrap();
    let spawned = recv_for_thread(&mut events, thread_id).await;
    assert_eq!(spawned.event, "subagent_spawned");

    tx.send(AgentProgress::SubagentCompleted {
        agent_id: "researcher".into(),
        task_id: "sub-c1-done".into(),
        elapsed_ms: 10,
        iterations: 1,
        output_chars: 5,
        usage: None,
        output: "final answer".into(),
        worktree_path: None,
        changed_files: Vec::new(),
        dirty_status: None,
        stop: None,
    })
    .await
    .unwrap();

    let completed = recv_for_thread(&mut events, thread_id).await;
    assert_eq!(completed.event, "subagent_completed");
    let subagent = completed.subagent.expect("subagent detail present");
    assert_eq!(subagent.parent_call_id.as_deref(), Some("call-parent-2"));
    assert_eq!(subagent.output.as_deref(), Some("final answer"));
}

#[tokio::test]
async fn subagent_failed_carries_parent_call_id() {
    let mut events = super::super::super::event_bus::subscribe_web_channel_events();
    let thread_id = "thread-c1-failed";
    let tx = spawn_test_bridge(thread_id, "req-c1-failed");

    tx.send(AgentProgress::SubagentSpawned {
        agent_id: "researcher".into(),
        task_id: "sub-c1-failed".into(),
        mode: "typed".into(),
        dedicated_thread: false,
        prompt_chars: 4,
        prompt: "help".into(),
        worker_thread_id: None,
        display_name: None,
        parent_call_id: Some("call-parent-3".into()),
    })
    .await
    .unwrap();
    let spawned = recv_for_thread(&mut events, thread_id).await;
    assert_eq!(spawned.event, "subagent_spawned");

    tx.send(AgentProgress::SubagentFailed {
        agent_id: "researcher".into(),
        task_id: "sub-c1-failed".into(),
        error: "boom".into(),
    })
    .await
    .unwrap();

    let failed = recv_for_thread(&mut events, thread_id).await;
    assert_eq!(failed.event, "subagent_failed");
    let subagent = failed.subagent.expect("subagent detail present");
    assert_eq!(subagent.parent_call_id.as_deref(), Some("call-parent-3"));
}
