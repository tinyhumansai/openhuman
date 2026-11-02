//! JSON-RPC E2E cases for the threads namespace.

#[path = "support/rpc_e2e_harness.rs"]
mod harness;
use harness::*;

#[tokio::test]
async fn json_rpc_thread_labels_create_and_update() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_url_guard = EnvVarGuard::unset("VITE_BACKEND_URL");
    let _api_url_guard = EnvVarGuard::unset("OPENHUMAN_API_URL");

    let (api_addr, api_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let api_origin = format!("http://{api_addr}");
    write_min_config(openhuman_home.as_path(), &api_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{rpc_addr}");

    // 1. Create a thread with an explicit label.
    let create = post_json_rpc(
        &rpc_base,
        9001,
        "openhuman.threads_create_new",
        json!({ "labels": ["custom"] }),
    )
    .await;
    let create_outer = assert_no_jsonrpc_error(&create, "threads_create_new with labels");
    let created = create_outer
        .get("data")
        .expect("data envelope in create response");
    let thread_id = created
        .get("id")
        .and_then(Value::as_str)
        .expect("id in created thread");
    let created_labels = created
        .get("labels")
        .and_then(Value::as_array)
        .expect("labels in created thread");
    assert_eq!(
        created_labels
            .iter()
            .map(|v| v.as_str().unwrap_or(""))
            .collect::<Vec<_>>(),
        vec!["custom"],
        "created thread should have labels=[\"custom\"]"
    );

    // 2. Update labels on the thread. Legacy "work" input normalizes to
    // "general" for backward compatibility with older callers.
    let update = post_json_rpc(
        &rpc_base,
        9002,
        "openhuman.threads_update_labels",
        json!({ "thread_id": thread_id, "labels": ["work", "briefing"] }),
    )
    .await;
    let update_outer = assert_no_jsonrpc_error(&update, "threads_update_labels");
    let updated = update_outer
        .get("data")
        .expect("data envelope in update response");
    let updated_labels = updated
        .get("labels")
        .and_then(Value::as_array)
        .expect("labels in updated thread");
    assert_eq!(
        updated_labels
            .iter()
            .map(|v| v.as_str().unwrap_or(""))
            .collect::<Vec<_>>(),
        vec!["general", "briefing"],
        "updated thread should normalize legacy work label to general"
    );

    // 3. Verify the updated labels are reflected in threads_list.
    let list = post_json_rpc(&rpc_base, 9003, "openhuman.threads_list", json!({})).await;
    let list_outer = assert_no_jsonrpc_error(&list, "threads_list after label update");
    let list_result = list_outer
        .get("data")
        .expect("data envelope in list response");
    let threads = list_result
        .get("threads")
        .and_then(Value::as_array)
        .expect("threads array in list");
    let persisted = threads
        .iter()
        .find(|t| t.get("id").and_then(Value::as_str) == Some(thread_id))
        .expect("created thread must appear in list");
    let persisted_labels = persisted
        .get("labels")
        .and_then(Value::as_array)
        .expect("labels in persisted thread");
    assert_eq!(
        persisted_labels
            .iter()
            .map(|v| v.as_str().unwrap_or(""))
            .collect::<Vec<_>>(),
        vec!["general", "briefing"],
        "threads_list must reflect the updated labels"
    );

    api_join.abort();
    rpc_join.abort();
}

/// `threads.goal_get` / `threads.todos_get` are the one-shot reads a client
/// makes to hydrate the goal chip / todo drawer for a thread that has neither
/// yet (both surfaces otherwise only stream live via `thread_goal_updated` /
/// `thread_todos_changed`). `channel.web_queue_remove` on an id that isn't
/// queued (no active turn at all, here) is a no-op, not an error — see C3.

#[tokio::test]
async fn json_rpc_thread_goal_and_todos_get_and_queue_remove_are_wired() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_url_guard = EnvVarGuard::unset("VITE_BACKEND_URL");
    let _api_url_guard = EnvVarGuard::unset("OPENHUMAN_API_URL");

    let (api_addr, api_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let api_origin = format!("http://{api_addr}");
    write_min_config(openhuman_home.as_path(), &api_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{rpc_addr}");

    let create = post_json_rpc(&rpc_base, 9101, "openhuman.threads_create_new", json!({})).await;
    let create_outer = assert_no_jsonrpc_error(&create, "threads_create_new");
    let thread_id = create_outer
        .get("data")
        .and_then(|d| d.get("id"))
        .and_then(Value::as_str)
        .expect("id in created thread")
        .to_string();

    // No goal / todos exist yet for a freshly created thread.
    let goal = post_json_rpc(
        &rpc_base,
        9102,
        "openhuman.threads_goal_get",
        json!({ "thread_id": thread_id }),
    )
    .await;
    let goal_data = assert_no_jsonrpc_error(&goal, "threads_goal_get")
        .get("data")
        .expect("data envelope in goal_get response")
        .clone();
    assert!(
        goal_data.get("goal").is_none_or(Value::is_null),
        "a fresh thread has no goal: {goal_data}"
    );

    let todos = post_json_rpc(
        &rpc_base,
        9103,
        "openhuman.threads_todos_get",
        json!({ "thread_id": thread_id }),
    )
    .await;
    let todos_data = assert_no_jsonrpc_error(&todos, "threads_todos_get")
        .get("data")
        .expect("data envelope in todos_get response")
        .clone();
    assert_eq!(
        todos_data
            .get("todos")
            .and_then(Value::as_array)
            .expect("todos array"),
        &Vec::<Value>::new(),
        "a fresh thread has no todos: {todos_data}"
    );

    // No active turn on the thread, so removing any item id is a no-op.
    let remove = post_json_rpc(
        &rpc_base,
        9104,
        "openhuman.channel_web_queue_remove",
        json!({
            "client_id": "e2e-client",
            "thread_id": thread_id,
            "item_id": "no-such-item",
        }),
    )
    .await;
    let remove_result = assert_no_jsonrpc_error(&remove, "channel_web_queue_remove")
        .get("result")
        .expect("result envelope in queue_remove response")
        .clone();
    assert_eq!(remove_result.get("removed"), Some(&Value::Bool(false)));

    api_join.abort();
    rpc_join.abort();
}

#[tokio::test]
async fn json_rpc_thread_title_create_and_update() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_url_guard = EnvVarGuard::unset("VITE_BACKEND_URL");
    let _api_url_guard = EnvVarGuard::unset("OPENHUMAN_API_URL");

    let (api_addr, api_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let api_origin = format!("http://{api_addr}");
    write_min_config(openhuman_home.as_path(), &api_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{rpc_addr}");

    // 1. Create a thread.
    let create = post_json_rpc(&rpc_base, 9101, "openhuman.threads_create_new", json!({})).await;
    let create_outer = assert_no_jsonrpc_error(&create, "threads_create_new for title update");
    let created = create_outer
        .get("data")
        .expect("data envelope in create response");
    let thread_id = created
        .get("id")
        .and_then(Value::as_str)
        .expect("id in created thread");

    // 2. Update the title.
    let update = post_json_rpc(
        &rpc_base,
        9102,
        "openhuman.threads_update_title",
        json!({ "thread_id": thread_id, "title": "Invoice follow-up" }),
    )
    .await;
    let update_outer = assert_no_jsonrpc_error(&update, "threads_update_title");
    let updated = update_outer
        .get("data")
        .expect("data envelope in update response");
    assert_eq!(
        updated.get("title").and_then(Value::as_str),
        Some("Invoice follow-up"),
        "update response must carry the new title"
    );
    assert_eq!(
        updated.get("id").and_then(Value::as_str),
        Some(thread_id),
        "update response must carry the thread id"
    );

    // 3. Verify the new title is reflected in threads_list.
    let list = post_json_rpc(&rpc_base, 9103, "openhuman.threads_list", json!({})).await;
    let list_outer = assert_no_jsonrpc_error(&list, "threads_list after title update");
    let threads = list_outer
        .get("data")
        .and_then(|d| d.get("threads"))
        .and_then(Value::as_array)
        .expect("threads array in list");
    let persisted = threads
        .iter()
        .find(|t| t.get("id").and_then(Value::as_str) == Some(thread_id))
        .expect("created thread must appear in list");
    assert_eq!(
        persisted.get("title").and_then(Value::as_str),
        Some("Invoice follow-up"),
        "threads_list must reflect the updated title"
    );

    // 4. Empty title is rejected.
    let bad = post_json_rpc(
        &rpc_base,
        9104,
        "openhuman.threads_update_title",
        json!({ "thread_id": thread_id, "title": "" }),
    )
    .await;
    let bad_err = assert_jsonrpc_error(&bad, "threads_update_title with empty title");
    let err_message = bad_err
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("error object missing message: {bad_err}"));
    assert!(
        err_message.contains("must not be empty"),
        "expected empty-title error, got: {err_message}"
    );

    api_join.abort();
    rpc_join.abort();
}

#[tokio::test]
async fn json_rpc_thread_working_dir_binds_before_the_first_message() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");
    let project = home.join("projects").join("site");
    std::fs::create_dir_all(&project).expect("project dir");
    let project = project.canonicalize().expect("canonical project");
    let project_str = project.to_string_lossy().into_owned();

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_url_guard = EnvVarGuard::unset("VITE_BACKEND_URL");
    let _api_url_guard = EnvVarGuard::unset("OPENHUMAN_API_URL");

    let (api_addr, api_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let api_origin = format!("http://{api_addr}");
    write_min_config(openhuman_home.as_path(), &api_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{rpc_addr}");

    // 1. A thread created in a folder carries it.
    let create = post_json_rpc(
        &rpc_base,
        9201,
        "openhuman.threads_create_new",
        json!({ "action_dir": project_str }),
    )
    .await;
    let created = assert_no_jsonrpc_error(&create, "threads_create_new with action_dir")
        .get("data")
        .cloned()
        .expect("data envelope");
    let thread_id = created
        .get("id")
        .and_then(Value::as_str)
        .expect("thread id")
        .to_string();
    assert_eq!(
        created.get("actionDir").and_then(Value::as_str),
        Some(project_str.as_str())
    );

    // 2. Clearing it on the still-empty thread goes back to the default.
    let cleared = post_json_rpc(
        &rpc_base,
        9202,
        "openhuman.threads_update_working_dir",
        json!({ "thread_id": thread_id, "action_dir": "" }),
    )
    .await;
    let cleared = assert_no_jsonrpc_error(&cleared, "threads_update_working_dir clear");
    assert!(cleared
        .get("data")
        .and_then(|d| d.get("actionDir"))
        .is_none());

    // 3. A protected folder is refused.
    let ssh = home.join(".ssh");
    std::fs::create_dir_all(&ssh).expect("ssh dir");
    let refused = post_json_rpc(
        &rpc_base,
        9203,
        "openhuman.threads_update_working_dir",
        json!({ "thread_id": thread_id, "action_dir": ssh.to_string_lossy() }),
    )
    .await;
    assert_jsonrpc_error(&refused, "threads_update_working_dir protected folder");

    // 4. Once the conversation has a message, the folder is fixed.
    let append = post_json_rpc(
        &rpc_base,
        9204,
        "openhuman.threads_message_append",
        json!({
            "thread_id": thread_id,
            "message": {
                "id": "m1",
                "content": "hello",
                "type": "text",
                "extraMetadata": {},
                "sender": "user",
                "createdAt": "2026-10-06T12:00:00Z"
            }
        }),
    )
    .await;
    assert_no_jsonrpc_error(&append, "threads_message_append");
    let late = post_json_rpc(
        &rpc_base,
        9205,
        "openhuman.threads_update_working_dir",
        json!({ "thread_id": thread_id, "action_dir": project_str }),
    )
    .await;
    let late_err = assert_jsonrpc_error(&late, "threads_update_working_dir after a message");
    let message = late_err
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(
        message.contains("before the conversation's first message"),
        "expected started-thread refusal, got: {message}"
    );

    api_join.abort();
    rpc_join.abort();
}

#[tokio::test]
async fn json_rpc_thread_not_found_errors_are_structured() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_url_guard = EnvVarGuard::unset("VITE_BACKEND_URL");
    let _api_url_guard = EnvVarGuard::unset("OPENHUMAN_API_URL");

    let (api_addr, api_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let api_origin = format!("http://{api_addr}");
    write_min_config(openhuman_home.as_path(), &api_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{rpc_addr}");
    let thread_id = "thread-missing";

    let append = post_json_rpc(
        &rpc_base,
        9011,
        "openhuman.threads_message_append",
        json!({
            "thread_id": thread_id,
            "message": {
                "id": "msg-1",
                "content": "hello",
                "type": "text",
                "extraMetadata": {},
                "sender": "user",
                "createdAt": "2026-01-01T00:00:00Z"
            }
        }),
    )
    .await;
    let append_err = assert_jsonrpc_error(&append, "threads_message_append missing thread");
    assert_eq!(append_err["message"], "thread thread-missing not found");
    assert_eq!(append_err["data"]["kind"], "ThreadNotFound");
    assert_eq!(append_err["data"]["thread_id"], thread_id);
    // The transport layer no longer stamps the RPC method into the structured
    // error data — the domain controller emits a method-agnostic envelope and
    // openhuman-rpc/src/server/http/rpc_handler.rs surfaces it verbatim. The frontend keys on `kind` +
    // `thread_id` (see `coreRpcClient.isThreadNotFoundRpcData`), not method.
    assert!(
        append_err["data"]["method"].is_null(),
        "method must not appear in structured error data: the domain envelope is method-agnostic"
    );

    let title = post_json_rpc(
        &rpc_base,
        9012,
        "openhuman.threads_generate_title",
        json!({ "thread_id": thread_id }),
    )
    .await;
    let title_err = assert_jsonrpc_error(&title, "threads_generate_title missing thread");
    assert_eq!(title_err["message"], "thread thread-missing not found");
    assert_eq!(title_err["data"]["kind"], "ThreadNotFound");
    assert_eq!(title_err["data"]["thread_id"], thread_id);

    api_join.abort();
    rpc_join.abort();
}

#[tokio::test]
async fn json_rpc_thread_generate_title_falls_back_when_provider_path_is_unavailable() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_url_guard = EnvVarGuard::unset("VITE_BACKEND_URL");
    let _api_url_guard = EnvVarGuard::unset("OPENHUMAN_API_URL");

    with_chat_completion_models(|models| models.clear());
    with_chat_completion_requests(|requests| requests.clear());

    let (api_addr, api_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let api_origin = format!("http://{api_addr}");
    write_min_config(openhuman_home.as_path(), &api_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{rpc_addr}");

    let create = post_json_rpc(&rpc_base, 9013, "openhuman.threads_create_new", json!({})).await;
    let create_outer = assert_no_jsonrpc_error(&create, "threads_create_new");
    let created = create_outer
        .get("data")
        .expect("data envelope in create response");
    let thread_id = created
        .get("id")
        .and_then(Value::as_str)
        .expect("thread id");
    let original_title = created
        .get("title")
        .and_then(Value::as_str)
        .expect("placeholder title")
        .to_string();

    let user_append = post_json_rpc(
        &rpc_base,
        9014,
        "openhuman.threads_message_append",
        json!({
            "thread_id": thread_id,
            "message": {
                "id": "msg-user",
                "content": "Please summarize the latest five email threads for me.",
                "type": "text",
                "extraMetadata": {},
                "sender": "user",
                "createdAt": "2026-01-01T00:00:00Z"
            }
        }),
    )
    .await;
    assert_no_jsonrpc_error(&user_append, "threads_message_append user");

    let agent_append = post_json_rpc(
        &rpc_base,
        9015,
        "openhuman.threads_message_append",
        json!({
            "thread_id": thread_id,
            "message": {
                "id": "msg-agent",
                "content": "Here is the summary you asked for.",
                "type": "text",
                "extraMetadata": {},
                "sender": "agent",
                "createdAt": "2026-01-01T00:00:02Z"
            }
        }),
    )
    .await;
    assert_no_jsonrpc_error(&agent_append, "threads_message_append agent");

    let title = post_json_rpc(
        &rpc_base,
        9016,
        "openhuman.threads_generate_title",
        json!({ "thread_id": thread_id }),
    )
    .await;
    let title_outer = assert_no_jsonrpc_error(&title, "threads_generate_title");
    let titled = title_outer
        .get("data")
        .expect("data envelope in title response");
    let generated_title = titled
        .get("title")
        .and_then(Value::as_str)
        .expect("generated title");

    assert_ne!(generated_title, original_title);
    assert_eq!(
        generated_title,
        "summarize latest five",
        "fallback title should be the 3-word shape derived from the first user message (filler stripped), got: {generated_title}"
    );

    let captured_models = with_chat_completion_models(|models| models.clone());
    assert!(
        captured_models.is_empty(),
        "the minimal config path currently falls back before hitting mock chat completions"
    );

    api_join.abort();
    rpc_join.abort();
}

#[tokio::test]
async fn json_rpc_thread_turn_state_lifecycle() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_url_guard = EnvVarGuard::unset("VITE_BACKEND_URL");
    let _api_url_guard = EnvVarGuard::unset("OPENHUMAN_API_URL");

    let (api_addr, api_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let api_origin = format!("http://{api_addr}");
    write_min_config(openhuman_home.as_path(), &api_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{rpc_addr}");

    // Empty workspace → no snapshots.
    let empty_list = post_json_rpc(
        &rpc_base,
        9101,
        "openhuman.threads_turn_state_list",
        json!({}),
    )
    .await;
    let outer = assert_no_jsonrpc_error(&empty_list, "turn_state_list (empty)");
    assert_eq!(
        outer
            .get("data")
            .and_then(|d| d.get("count"))
            .and_then(serde_json::Value::as_u64),
        Some(0)
    );

    // Drop a snapshot directly through the store — this is exactly what
    // the web-channel progress mirror does mid-turn.
    let workspace_dir = {
        let cfg = openhuman_core::config::Config::load_or_init()
            .await
            .expect("load config");
        cfg.workspace_dir
    };
    let mut state = tinyagents_session::turn_state::TurnState::started(
        "thread-turn-1",
        "req-turn-1",
        25,
        chrono::Utc::now().to_rfc3339(),
    );
    state.lifecycle = tinyagents_session::turn_state::TurnLifecycle::Streaming;
    state.iteration = 2;
    state.streaming_text = "partial".into();
    openhuman_core::storage::sessions::turn_states_for(&workspace_dir)
        .put(&state)
        .expect("seed snapshot");

    // get → present
    let got = post_json_rpc(
        &rpc_base,
        9102,
        "openhuman.threads_turn_state_get",
        json!({ "thread_id": "thread-turn-1" }),
    )
    .await;
    let got_outer = assert_no_jsonrpc_error(&got, "turn_state_get (present)");
    let payload = got_outer
        .get("data")
        .and_then(|d| d.get("turnState"))
        .expect("turnState present");
    assert_eq!(
        payload.get("threadId").and_then(serde_json::Value::as_str),
        Some("thread-turn-1")
    );
    assert_eq!(
        payload.get("lifecycle").and_then(serde_json::Value::as_str),
        Some("streaming")
    );
    assert_eq!(
        payload.get("iteration").and_then(serde_json::Value::as_u64),
        Some(2)
    );

    // list → contains the seeded snapshot
    let list = post_json_rpc(
        &rpc_base,
        9103,
        "openhuman.threads_turn_state_list",
        json!({}),
    )
    .await;
    let list_outer = assert_no_jsonrpc_error(&list, "turn_state_list (one)");
    assert_eq!(
        list_outer
            .get("data")
            .and_then(|d| d.get("count"))
            .and_then(serde_json::Value::as_u64),
        Some(1)
    );

    // Seed a second, later turn on the same thread — the per-turn ring keeps
    // both instead of overwriting.
    // Far-future started_at guarantees turn-2 is the newest (turn-1 was seeded
    // with the real `now()`), so history ordering is deterministic.
    let mut state2 = tinyagents_session::turn_state::TurnState::started(
        "thread-turn-1",
        "req-turn-2",
        25,
        "2999-01-01T00:00:00Z",
    );
    state2.lifecycle = tinyagents_session::turn_state::TurnLifecycle::Completed;
    state2.updated_at = "2999-01-01T00:00:00Z".into();
    openhuman_core::storage::sessions::turn_states_for(&workspace_dir)
        .put(&state2)
        .expect("seed snapshot 2");

    // history → both turns, newest first.
    let history = post_json_rpc(
        &rpc_base,
        9106,
        "openhuman.threads_turn_state_history",
        json!({ "thread_id": "thread-turn-1" }),
    )
    .await;
    let history_outer = assert_no_jsonrpc_error(&history, "turn_state_history");
    let turns = history_outer
        .get("data")
        .and_then(|d| d.get("turnStates"))
        .and_then(serde_json::Value::as_array)
        .expect("turnStates array");
    assert_eq!(turns.len(), 2, "both turns retained");
    assert_eq!(
        turns[0]
            .get("requestId")
            .and_then(serde_json::Value::as_str),
        Some("req-turn-2"),
        "newest turn first"
    );

    // get_turn → the specific earlier turn.
    let get_turn = post_json_rpc(
        &rpc_base,
        9107,
        "openhuman.threads_turn_state_get_turn",
        json!({ "thread_id": "thread-turn-1", "request_id": "req-turn-1" }),
    )
    .await;
    let get_turn_outer = assert_no_jsonrpc_error(&get_turn, "turn_state_get_turn (present)");
    assert_eq!(
        get_turn_outer
            .get("data")
            .and_then(|d| d.get("turnState"))
            .and_then(|t| t.get("requestId"))
            .and_then(serde_json::Value::as_str),
        Some("req-turn-1")
    );

    // get_turn for an unknown request → null.
    let get_turn_missing = post_json_rpc(
        &rpc_base,
        9108,
        "openhuman.threads_turn_state_get_turn",
        json!({ "thread_id": "thread-turn-1", "request_id": "nope" }),
    )
    .await;
    let missing_outer = assert_no_jsonrpc_error(&get_turn_missing, "turn_state_get_turn (absent)");
    assert!(missing_outer
        .get("data")
        .and_then(|d| d.get("turnState"))
        .map(|v| v.is_null())
        .unwrap_or(true));

    // clear → cleared:true
    let cleared = post_json_rpc(
        &rpc_base,
        9104,
        "openhuman.threads_turn_state_clear",
        json!({ "thread_id": "thread-turn-1" }),
    )
    .await;
    let cleared_outer = assert_no_jsonrpc_error(&cleared, "turn_state_clear");
    assert_eq!(
        cleared_outer
            .get("data")
            .and_then(|d| d.get("cleared"))
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );

    // subsequent get returns null
    let got_again = post_json_rpc(
        &rpc_base,
        9105,
        "openhuman.threads_turn_state_get",
        json!({ "thread_id": "thread-turn-1" }),
    )
    .await;
    let again_outer = assert_no_jsonrpc_error(&got_again, "turn_state_get (after clear)");
    assert!(again_outer
        .get("data")
        .and_then(|d| d.get("turnState"))
        .map(|v| v.is_null())
        .unwrap_or(true));

    api_join.abort();
    rpc_join.abort();
}

#[tokio::test]
async fn json_rpc_threads_token_usage_reads_persisted_thread_totals() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");
    let workspace = home.join("workspace");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", home);
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");

    write_min_config(&openhuman_home, "http://127.0.0.1:9");

    // Plant a root session transcript for thread "thr-e2e": two turns, each
    // carrying its own `usage` record, and a `_meta` header whose token rollups
    // are ZERO.
    //
    // The zeros are the point. Every root transcript written since the TinyAgents
    // runtime cutover (`33566d382`) has a zero `_meta` rollup while its per-turn
    // records hold the real numbers (#6460). This fixture used to carry a
    // non-zero `_meta` and no per-turn records on the sub-agent at all, so it
    // asserted a contract the running system never produced — the RPC passed
    // here while reading zeros in production. Totals below can now only come
    // from the per-turn records, which is what the core actually writes.
    //
    // The two turns sum to the same 4200/900/600 the old header claimed, so every
    // contract assertion further down (including the cost arithmetic) is
    // unchanged; only the provenance of the numbers moved.
    //
    // `provider`, `model`, `usage` and `ts` are ALL load-bearing on an assistant
    // row: `turn_usage_from_line` matches on all four being present and yields
    // `None` if any is missing, so dropping one silently produces a row the
    // model-context reader sees no usage on. The real writer always emits all
    // four (verified against every usage-bearing row in a live transcript). The
    // previous fixture omitted `provider` and still passed, because
    // `read_last_assistant_usage` — the only reader it exercised — is more
    // lenient than this one; that is the same "green over a shape production
    // never writes" defect as the zero `_meta` above.
    let raw = workspace.join("session_raw");
    std::fs::create_dir_all(&raw).expect("mkdir session_raw");
    let jsonl = format!(
        "{}\n{}\n{}\n{}\n{}\n",
        json!({"_meta": {
            "agent": "orchestrator", "dispatcher": "native",
            "created": "2026-04-11T14:30:00Z", "updated": "2026-04-11T14:35:22Z",
            "turn_count": 2, "input_tokens": 0, "output_tokens": 0,
            "cached_input_tokens": 0, "charged_amount_usd": 0.0, "thread_id": "thr-e2e"
        }}),
        json!({"role": "user", "content": "hi"}),
        json!({"role": "assistant", "content": "first",
            "provider": "openhuman", "model": "reasoning-v1",
            "usage": {"input": 3850, "output": 820, "cached_input": 560, "cost_usd": 0.0114},
            "ts": "2026-04-11T14:32:00Z"}),
        json!({"role": "user", "content": "again"}),
        json!({"role": "assistant", "content": "hello",
            "provider": "openhuman", "model": "reasoning-v1",
            "usage": {"input": 350, "output": 80, "cached_input": 40, "cost_usd": 0.0009},
            "ts": "2026-04-11T14:35:22Z"}),
    );
    let root_stem = tinyagents_session::transcript::session_stem(
        &tinyagents_session::transcript::SessionRef::scoped("thr-e2e", "orchestrator"),
    );
    std::fs::write(raw.join(format!("{root_stem}.jsonl")), jsonl).expect("write transcript");

    // A sub-agent transcript (stem contains `__`) for the SAME thread: a `coder`
    // archetype. Its message carries NO model (mirroring how sub-agent
    // transcripts were historically written), so pricing must fall back to the
    // thread's model rather than $0. Its spend is grouped under `coder` and
    // folded into the thread totals — not collapsed into the orchestrator.
    //
    // Its `thread_id` is a WORKER thread, not "thr-e2e". That is what a real
    // delegation writes — `inherited_thread_id` lets the worker id win over the
    // parent's — and selecting children by thread id therefore dropped them
    // entirely (#6460). They are found by the `{root_stem}__` filename relation
    // instead, which is what this stem exercises. Its `_meta` rollups are zero
    // like the root's, so the 1000/200 below can only come from its own turn
    // record.
    //
    // Its `model` is deliberately the EMPTY STRING rather than absent. `model`
    // has to be present for `turn_usage_from_line` to yield usage at all, and an
    // empty one is exactly what the codec writes when no provider route resolved
    // (`route.map(|r| r.model).unwrap_or_default()`), which is the real shape of
    // a child that never recorded a model. The aggregate ignores an empty model,
    // so pricing still falls back to the thread's model below rather than $0 —
    // the behaviour this case has always been here to pin.
    let sub_jsonl = format!(
        "{}\n{}\n",
        json!({"_meta": {
            "agent": "coder", "dispatcher": "native",
            "created": "2026-04-11T14:33:00Z", "updated": "2026-04-11T14:34:00Z",
            "turn_count": 1, "input_tokens": 0, "output_tokens": 0,
            "cached_input_tokens": 0, "charged_amount_usd": 0.0,
            "thread_id": "worker-2f1c9a80-4d55-4a7e-9f11-0c6b5d3e8a42"
        }}),
        json!({"role": "assistant", "content": "done",
            "provider": "openhuman", "model": "",
            "usage": {"input": 1000, "output": 200, "cached_input": 0, "cost_usd": 0.0},
            "ts": "2026-04-11T14:34:00Z"}),
    );
    std::fs::write(
        raw.join(format!("{root_stem}__1700000050_coder.jsonl")),
        sub_jsonl,
    )
    .expect("write subagent transcript");

    // The host imports classic workspace transcripts during runtime boot.
    // This test builds the router directly, so run the same importer before
    // asking the storage-backed thread API to read the seeded records.
    let config = openhuman_core::config::Config::load_or_init()
        .await
        .expect("load test config");
    assert_eq!(config.workspace_dir, workspace);
    openhuman_core::agent_registry::host_definition::AgentDefinitionRegistry::init_global_builtins(
    )
    .expect("initialize built-in agent definitions");
    let backend = openhuman_core::storage::local::backend_for(&config.workspace_dir)
        .expect("open test workspace storage");
    let reports = openhuman_core::storage::import::sessions::run_with(&backend, &config);
    assert!(
        matches!(
            reports.first().map(|report| &report.outcome),
            Some(Ok(openhuman_core::storage::import::Outcome::Imported {
                found: 2,
                written: 2
            }))
        ),
        "import classic session transcripts: {reports:?}"
    );
    let imported = openhuman_core::storage::sessions::provider_for_workspace(&config.workspace_dir)
        .for_agent("orchestrator");
    let spend = tinyagents_session::transcript::spend::thread_spend_in(
        imported.transcripts.as_ref(),
        "thr-e2e",
    );
    assert_eq!(
        spend.root.input_tokens, 4200,
        "imported root spend: {spend:?}"
    );

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{rpc_addr}");

    // Known thread → totals read back from the transcripts (orchestrator + coder).
    let resp = post_json_rpc(
        &rpc_base,
        1,
        "openhuman.threads_token_usage",
        json!({ "thread_id": "thr-e2e" }),
    )
    .await;
    let envelope = assert_no_jsonrpc_error(&resp, "threads_token_usage");
    let data = envelope
        .get("data")
        .unwrap_or_else(|| panic!("missing data envelope: {envelope}"));
    // Top-level totals include the sub-agent: 4200+1000 in, 900+200 out, each
    // counted exactly once — the root's records are its OWN spend, the child's
    // are the child's.
    assert_eq!(data["input_tokens"], 5200);
    assert_eq!(data["output_tokens"], 1100);
    assert_eq!(data["cached_input_tokens"], 600);
    // These records predate `cost_source`, so their persisted cost may be a
    // guessed rate and is not reported. `reasoning-v1` (a retired tier slug,
    // and the coder's fallback model) has no catalogued list price either, so
    // the thread's cost is unknown: `null`, never a default rate.
    assert!(data["cost_usd"].is_null(), "cost_usd: {}", data["cost_usd"]);
    assert_eq!(data["cost_source"], "unknown");
    assert_eq!(data["turn_count"], 2);
    assert_eq!(data["last_turn_input_tokens"], 350);
    assert_eq!(data["last_turn_output_tokens"], 80);
    // These records predate the per-call fields and carry no call count, so
    // the gauge numerator falls back to the one-call reading: 350 + 80.
    assert_eq!(data["last_turn_context_tokens"], 430);
    assert_eq!(data["model"], "reasoning-v1");
    // reasoning-v1 resolves to a 1M context window.
    assert_eq!(data["context_window"], 1_000_000);
    assert_eq!(data["has_usage"], true);
    // Sub-agent breakdown grouped by archetype.
    let subs = data["subagents"].as_array().expect("subagents array");
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0]["agent_id"], "coder");
    assert_eq!(subs[0]["input_tokens"], 1000);
    assert_eq!(subs[0]["output_tokens"], 200);
    assert_eq!(subs[0]["runs"], 1);
    assert!(
        subs[0]["cost_usd"].is_null(),
        "an unpriced sub-agent reports no cost"
    );

    // Unknown thread → all-zero totals with has_usage=false (brand-new thread).
    let resp_unknown = post_json_rpc(
        &rpc_base,
        2,
        "openhuman.threads_token_usage",
        json!({ "thread_id": "thr-does-not-exist" }),
    )
    .await;
    let unknown = assert_no_jsonrpc_error(&resp_unknown, "threads_token_usage unknown");
    let unknown_data = unknown
        .get("data")
        .unwrap_or_else(|| panic!("missing data envelope: {unknown}"));
    assert_eq!(unknown_data["input_tokens"], 0);
    assert_eq!(unknown_data["cost_usd"], 0.0);
    assert_eq!(unknown_data["has_usage"], false);

    rpc_join.abort();
}

/// Seed a raw append-only session transcript at
/// `{workspace}/session_raw/{stem}.jsonl` (meta header + body lines).
fn seed_raw_transcript(workspace: &Path, stem: &str, thread_id: &str, body: &[&str]) {
    let raw_dir = workspace.join("session_raw");
    std::fs::create_dir_all(&raw_dir).expect("create session_raw");
    let meta = format!(
        r#"{{"_meta":{{"version":1,"agent":"orchestrator","dispatcher":"native","created":"2026-07-21T00:00:00Z","updated":"2026-07-21T00:00:10Z","turn_count":1,"input_tokens":30,"output_tokens":13,"cached_input_tokens":0,"charged_amount_usd":0.003,"thread_id":"{thread_id}"}}}}"#
    );
    let mut buf = meta;
    buf.push('\n');
    for line in body {
        buf.push_str(line);
        buf.push('\n');
    }
    std::fs::write(raw_dir.join(format!("{stem}.jsonl")), buf).expect("write transcript");
}

#[tokio::test]
async fn json_rpc_threads_transcript_get_projects_and_paginates() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    // Config resolves the runtime workspace to `OPENHUMAN_WORKSPACE/workspace`
    // (see resolve_config_dir_for_workspace), so seed transcripts there.
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    let workspace = workspace.as_path();

    let _workspace_guard = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", tmp.path());
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_url_guard = EnvVarGuard::unset("VITE_BACKEND_URL");
    let _api_url_guard = EnvVarGuard::unset("OPENHUMAN_API_URL");

    let thread_id = "thr_transcript_e2e";
    // The importer files a transcript by the session key and agent component
    // its stem carries (`<session>.<agent>`); a bare legacy stem lands in the
    // default agent, which is not where the thread API looks.
    let root_stem = tinyagents_session::transcript::session_stem(
        &tinyagents_session::transcript::SessionRef::scoped(thread_id, "orchestrator"),
    );
    let root_stem = root_stem.as_str();
    // A full turn: system scaffolding (dropped), user with injected datetime
    // prefix, assistant tool-calling step (reasoning + tool_calls), tool result,
    // final assistant answer.
    seed_raw_transcript(
        workspace,
        root_stem,
        thread_id,
        &[
            r#"{"role":"system","content":"[tool-policy preamble] ..."}"#,
            r#"{"role":"user","content":"Current Date & Time: 2026-07-21 09:00:00 UTC\n\nWeather in NYC?","request_id":"req-1"}"#,
            r#"{"role":"assistant","content":"Checking.","provider":"anthropic","model":"claude-x","usage":{"input":10,"output":5,"cached_input":0,"cost_usd":0.001},"ts":"2026-07-21T09:00:01Z","reasoning_content":"call the tool","tool_calls":[{"id":"call-1","name":"get_weather","arguments":"{\"city\":\"NYC\"}"},{"id":"call-2","name":"get_traffic","arguments":"{\"city\":\"NYC\"}"}],"iteration":1,"request_id":"req-1"}"#,
            r#"{"role":"tool","content":"72F sunny","id":"call-1","request_id":"req-1"}"#,
            r#"{"role":"tool","content":"error: traffic service unavailable","id":"call-2","request_id":"req-1","failure":true,"failure_detail":"traffic service unavailable"}"#,
            r#"{"role":"assistant","content":"72F and sunny.","provider":"anthropic","model":"claude-x","usage":{"input":20,"output":8,"cached_input":0,"cost_usd":0.002},"ts":"2026-07-21T09:00:02Z","iteration":2,"request_id":"req-1"}"#,
        ],
    );
    // A sub-agent sibling sharing the root stem.
    seed_raw_transcript(
        workspace,
        &format!("{root_stem}__50_coder"),
        thread_id,
        &[
            r#"{"role":"assistant","content":"sub work done","provider":"anthropic","model":"claude-x","usage":{"input":5,"output":3,"cached_input":0,"cost_usd":0.0},"ts":"2026-07-21T09:10:00Z","iteration":1}"#,
        ],
    );

    // The host imports classic workspace transcripts during runtime boot.
    // This test builds the router directly, so import the seeded records before
    // asking the storage-backed thread API to read them.
    let config = openhuman_core::config::Config::load_or_init()
        .await
        .expect("load test config");
    assert_eq!(config.workspace_dir, workspace);
    let mut config = config;
    config.agent.chat_agent_id = Some("orchestrator".into());
    config.save().await.expect("save test chat agent");
    openhuman_core::agent_registry::host_definition::AgentDefinitionRegistry::init_global_builtins(
    )
    .expect("initialize built-in agent definitions");
    let backend = openhuman_core::storage::local::backend_for(&config.workspace_dir)
        .expect("open test workspace storage");
    let reports = openhuman_core::storage::import::sessions::run_with(&backend, &config);
    assert!(
        matches!(
            reports.first().map(|report| &report.outcome),
            Some(Ok(openhuman_core::storage::import::Outcome::Imported {
                found: 2,
                written: 2
            }))
        ),
        "import classic session transcripts: {reports:?}"
    );

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{rpc_addr}");

    // Full projection (default limit).
    let full = post_json_rpc(
        &rpc_base,
        41_001,
        "openhuman.threads_transcript_get",
        json!({ "thread_id": thread_id }),
    )
    .await;
    let full_result = assert_no_jsonrpc_error(&full, "threads_transcript_get");
    let data = full_result.get("data").expect("data envelope");
    assert_eq!(
        data.get("threadId").and_then(Value::as_str),
        Some(thread_id)
    );
    assert_eq!(
        data.get("hasTranscript").and_then(Value::as_bool),
        Some(true)
    );

    let items = data
        .get("items")
        .and_then(Value::as_array)
        .expect("items array");
    // 7 top-level from the root turn (turnBoundary, userMessage, reasoning,
    // interim assistant, 2 toolCalls, final assistant) + 1 subagent = 8.
    assert_eq!(
        data.get("total").and_then(Value::as_u64),
        Some(8),
        "items: {items:#?}"
    );

    let kinds: Vec<&str> = items
        .iter()
        .filter_map(|i| i.get("kind").and_then(Value::as_str))
        .collect();
    assert!(kinds.contains(&"turnBoundary"));
    assert!(kinds.contains(&"userMessage"));
    assert!(kinds.contains(&"reasoning"));
    assert!(kinds.contains(&"toolCall"));
    assert!(kinds.contains(&"subagent"));

    // Sanitization: the user message keeps raw content but exposes a stripped
    // displayContent.
    let user = items
        .iter()
        .find(|i| i.get("kind").and_then(Value::as_str) == Some("userMessage"))
        .expect("userMessage present");
    assert_eq!(
        user.get("displayContent").and_then(Value::as_str),
        Some("Weather in NYC?")
    );

    // Tool call paired with its result.
    let tool = items
        .iter()
        .find(|i| i.get("callId").and_then(Value::as_str) == Some("call-1"))
        .expect("toolCall call-1 present");
    assert_eq!(
        tool.get("result").and_then(Value::as_str),
        Some("72F sunny")
    );
    assert_eq!(tool.get("status").and_then(Value::as_str), Some("success"));
    assert!(
        tool.get("failure").is_none(),
        "successful tool has no failure"
    );

    // A failed tool result projects an error status + failure payload rather
    // than a false success (Gap 1).
    let failed_tool = items
        .iter()
        .find(|i| i.get("callId").and_then(Value::as_str) == Some("call-2"))
        .expect("toolCall call-2 present");
    assert_eq!(
        failed_tool.get("status").and_then(Value::as_str),
        Some("error")
    );
    assert_eq!(
        failed_tool
            .get("failure")
            .and_then(|f| f.get("detail"))
            .and_then(Value::as_str),
        Some("traffic service unavailable")
    );

    // Pagination: newest-first, limit 2 → 2 items + a cursor.
    let page1 = post_json_rpc(
        &rpc_base,
        41_002,
        "openhuman.threads_transcript_get",
        json!({ "thread_id": thread_id, "limit": 2 }),
    )
    .await;
    let page1_data = assert_no_jsonrpc_error(&page1, "transcript_get page1")
        .get("data")
        .cloned()
        .expect("data");
    assert_eq!(
        page1_data
            .get("items")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(2)
    );
    assert_eq!(
        page1_data.get("hasMore").and_then(Value::as_bool),
        Some(true)
    );
    let cursor = page1_data
        .get("nextCursor")
        .and_then(Value::as_str)
        .expect("nextCursor present")
        .to_string();

    let page2 = post_json_rpc(
        &rpc_base,
        41_003,
        "openhuman.threads_transcript_get",
        json!({ "thread_id": thread_id, "limit": 2, "cursor": cursor }),
    )
    .await;
    let page2_data = assert_no_jsonrpc_error(&page2, "transcript_get page2")
        .get("data")
        .cloned()
        .expect("data");
    assert_eq!(
        page2_data
            .get("items")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(2),
        "second page returns the next two items"
    );

    // Missing thread → empty page, not an error.
    let missing = post_json_rpc(
        &rpc_base,
        41_004,
        "openhuman.threads_transcript_get",
        json!({ "thread_id": "no_such_thread" }),
    )
    .await;
    let missing_data = assert_no_jsonrpc_error(&missing, "transcript_get missing")
        .get("data")
        .cloned()
        .expect("data");
    assert_eq!(
        missing_data.get("hasTranscript").and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(missing_data.get("total").and_then(Value::as_u64), Some(0));

    rpc_join.abort();
}

/// The `memory_diff` RPC surface is gone, and the `memory` domain is not (#5839).
///
/// #5839 deleted the `memory-git` feature, `crates/openhuman-core/src/memory/diff/`, the
/// `memory_diff` tool and `tests/memory_artifacts_e2e.rs`. What it left behind
/// is a unit test over `all_controller_schemas()`
/// (`crates/openhuman-core/src/core/all_tests.rs::memory_diff_controllers_are_gone_and_memory_survives`),
/// which reads the registry as a data structure. Nothing dispatched a removed
/// method through the live router, so nothing proved the wire surface actually
/// went with it — a re-registration behind a different namespace, or a stale
/// alias, would satisfy that unit test and still answer on `/rpc`.
///
/// This asks the real HTTP router. Both halves matter and are deliberately in
/// one test: removing the git ledger must not take the memory domain with it,
/// so a change that deleted too much fails here rather than passing quietly.

#[test]
fn json_rpc_threads_edit_message_truncates_and_restarts_turn() {
    run_json_rpc_e2e_on_agent_stack(
        "json_rpc_threads_edit_message_truncates_and_restarts_turn",
        json_rpc_threads_edit_message_truncates_and_restarts_turn_inner,
    );
}

async fn json_rpc_threads_edit_message_truncates_and_restarts_turn_inner() {
    tinyhumans_boot::boot();
    // `threads.edit_message` (wire method `openhuman.threads_edit_message`)
    // cancels any in-flight turn, forks the session transcript + message log
    // to drop the edited message and everything after it, then restarts the
    // turn with the new content. This round-trips a real completed turn
    // through the mock upstream, edits the user message that produced it
    // (which does have a reply, per the module's own doc comment on the
    // "editing the newest unanswered message" gap), and verifies the store
    // was actually mutated rather than just accepting the RPC.
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_url_guard = EnvVarGuard::unset("VITE_BACKEND_URL");
    let _api_url_guard = EnvVarGuard::unset("OPENHUMAN_API_URL");

    let (api_addr, api_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let api_origin = format!("http://{api_addr}");
    write_min_config(openhuman_home.as_path(), &api_origin);
    write_min_config(&openhuman_home.join("users").join("e2e-user"), &api_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{rpc_addr}");
    let store = post_json_rpc(
        &rpc_base,
        9599,
        "openhuman.auth_store_session",
        json!({ "token": "e2e-test-jwt", "user_id": "e2e-user" }),
    )
    .await;
    assert_no_jsonrpc_error(&store, "store_session before edit");

    let client_id = "e2e-edit-client";
    let create = post_json_rpc(&rpc_base, 9600, "openhuman.threads_create_new", json!({})).await;
    let thread_id = assert_no_jsonrpc_error(&create, "threads_create_new before edit")
        .get("data")
        .and_then(|data| data.get("id"))
        .and_then(Value::as_str)
        .expect("created thread id")
        .to_owned();
    let events_url = format!("{}/events?client_id={}", rpc_base, client_id);

    // The frontend — not the core — is the one that appends the user's own
    // message to the conversation store (the core only auto-persists the
    // assistant's reply, see `web_chat::reply_persistence`'s module doc), so
    // mirror that here *before* running the turn: the store append order
    // must be [user message, then its auto-persisted reply] for
    // `next_reply_request_id_after` to find the correlation edit_message
    // relies on.
    let user_message_id = "msg-user-edit-e2e";
    let user_append = post_json_rpc(
        &rpc_base,
        9602,
        "openhuman.threads_message_append",
        json!({
            "thread_id": thread_id,
            "message": {
                "id": user_message_id,
                "content": "Original message for edit test",
                "type": "text",
                "extraMetadata": {},
                "sender": "user",
                "createdAt": "2026-01-01T00:00:00Z"
            }
        }),
    )
    .await;
    assert_no_jsonrpc_error(&user_append, "threads_message_append user (pre-turn)");

    // --- Turn 1: a normal web-channel turn against the mock upstream, for
    // the same content just appended above. ---
    let (sse_task_1, request_tx) =
        spawn_ready_terminal_web_chat_event_for_request(&events_url).await;
    let turn1 = post_json_rpc(
        &rpc_base,
        9601,
        "openhuman.channel_web_chat",
        json!({
            "client_id": client_id,
            "thread_id": thread_id,
            "message": "Original message for edit test",
            "model_override": "e2e-mock-model",
        }),
    )
    .await;
    let turn1_result = assert_no_jsonrpc_error(&turn1, "channel_web_chat turn1");
    signal_accepted_web_chat_request_id(request_tx, turn1_result);
    let sse_event_1 = sse_task_1.await.expect("sse task 1 join should succeed");
    assert_eq!(
        sse_event_1.get("event").and_then(Value::as_str),
        Some("chat_done"),
        "turn1 should complete successfully: {sse_event_1}"
    );

    let before_list = post_json_rpc(
        &rpc_base,
        9603,
        "openhuman.threads_messages_list",
        json!({ "thread_id": thread_id }),
    )
    .await;
    let before_outer = assert_no_jsonrpc_error(&before_list, "threads_messages_list before edit");
    let before_data = before_outer
        .get("data")
        .expect("data envelope in messages_list response");
    let before_messages = before_data
        .get("messages")
        .and_then(Value::as_array)
        .expect("messages array before edit");
    let before_count = before_messages.len();
    assert!(
        before_messages
            .iter()
            .any(|m| m.get("id").and_then(Value::as_str) == Some(user_message_id)),
        "expected the manually-appended user message before editing: {before_messages:?}"
    );

    // --- Edit the user message: cancels (no-op, turn1 already finished),
    // forks the transcript before turn1's user prompt, truncates the message
    // log from `user_message_id` onward, and restarts with new content. ---
    let (sse_task_2, request_tx) =
        spawn_ready_terminal_web_chat_event_for_request(&events_url).await;
    let edit = post_json_rpc(
        &rpc_base,
        9604,
        "openhuman.threads_edit_message",
        json!({
            "thread_id": thread_id,
            "message_id": user_message_id,
            "content": "Edited message for edit test",
            "client_id": client_id,
        }),
    )
    .await;
    let edit_outer = assert_no_jsonrpc_error(&edit, "threads_edit_message");
    let edit_result = peel_logs_envelope(edit_outer);
    let new_request_id = edit_result
        .get("request_id")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("expected 'request_id' in edit_message response: {edit_outer}"));
    assert!(
        !new_request_id.is_empty(),
        "edit_message must return a non-empty request_id: {edit_outer}"
    );
    request_tx
        .send(new_request_id.to_owned())
        .expect("SSE reader should receive edited turn request_id");

    let sse_event_2 = sse_task_2.await.expect("sse task 2 join should succeed");
    assert_eq!(
        sse_event_2.get("event").and_then(Value::as_str),
        Some("chat_done"),
        "the restarted turn after edit should complete successfully: {sse_event_2}"
    );

    // --- Verify the store was actually mutated: the edited message and
    // everything after it (the old reply) are gone, replaced by the fresh
    // turn's own reply. ---
    let after_list = post_json_rpc(
        &rpc_base,
        9605,
        "openhuman.threads_messages_list",
        json!({ "thread_id": thread_id }),
    )
    .await;
    let after_outer = assert_no_jsonrpc_error(&after_list, "threads_messages_list after edit");
    let after_data = after_outer
        .get("data")
        .expect("data envelope in messages_list response");
    let after_messages = after_data
        .get("messages")
        .and_then(Value::as_array)
        .expect("messages array after edit");

    assert!(
        !after_messages
            .iter()
            .any(|m| m.get("id").and_then(Value::as_str) == Some(user_message_id)),
        "edited user message must be truncated from the log: {after_messages:?}"
    );
    assert!(
        after_messages.len() < before_count,
        "expected fewer messages after the edit truncation (before={before_count}, \
         after={}): {after_messages:?}",
        after_messages.len()
    );
    assert!(
        after_messages
            .iter()
            .any(|m| m.get("id").and_then(Value::as_str)
                == Some(format!("agent:{new_request_id}").as_str())),
        "expected the fresh turn's reply (agent:{new_request_id}) in the log: {after_messages:?}"
    );

    api_join.abort();
    rpc_join.abort();
}

#[test]
fn json_rpc_threads_regenerate_truncates_and_restarts_turn() {
    run_json_rpc_e2e_on_agent_stack(
        "json_rpc_threads_regenerate_truncates_and_restarts_turn",
        json_rpc_threads_regenerate_truncates_and_restarts_turn_inner,
    );
}

async fn json_rpc_threads_regenerate_truncates_and_restarts_turn_inner() {
    tinyhumans_boot::boot();
    // `threads.regenerate` (wire method `openhuman.threads_regenerate`) with
    // no `message_id` redoes the thread's last turn: cancels any in-flight
    // turn, forks the transcript at `TruncateCut::LastAssistantTurn`, drops
    // the old reply from the message log, then restarts with the same user
    // prompt. Verifies the old reply id is gone and a fresh one appears once
    // the new turn completes.
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_url_guard = EnvVarGuard::unset("VITE_BACKEND_URL");
    let _api_url_guard = EnvVarGuard::unset("OPENHUMAN_API_URL");

    let (api_addr, api_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let api_origin = format!("http://{api_addr}");
    write_min_config(openhuman_home.as_path(), &api_origin);
    write_min_config(&openhuman_home.join("users").join("e2e-user"), &api_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{rpc_addr}");

    let store = post_json_rpc(
        &rpc_base,
        9700,
        "openhuman.auth_store_session",
        json!({ "token": "e2e-test-jwt", "user_id": "e2e-user" }),
    )
    .await;
    assert_no_jsonrpc_error(&store, "store_session before regenerate");

    let client_id = "e2e-regen-client";
    let create = post_json_rpc(&rpc_base, 9699, "openhuman.threads_create_new", json!({})).await;
    let thread_id = assert_no_jsonrpc_error(&create, "threads_create_new before regenerate")
        .get("data")
        .and_then(|data| data.get("id"))
        .and_then(Value::as_str)
        .expect("created thread id")
        .to_owned();
    let events_url = format!("{}/events?client_id={}", rpc_base, client_id);

    // --- Turn 1: a normal web-channel turn against the mock upstream. ---
    let (sse_task_1, request_tx) =
        spawn_ready_terminal_web_chat_event_for_request(&events_url).await;
    let turn1 = post_json_rpc(
        &rpc_base,
        9701,
        "openhuman.channel_web_chat",
        json!({
            "client_id": client_id,
            "thread_id": thread_id,
            "message": "Original message for regenerate test",
            "model_override": "e2e-mock-model",
        }),
    )
    .await;
    let turn1_result = assert_no_jsonrpc_error(&turn1, "channel_web_chat turn1");
    let turn1_request_id = turn1_result
        .get("result")
        .and_then(|v| v.get("request_id"))
        .and_then(Value::as_str)
        .unwrap_or_else(|| {
            panic!("expected request_id in channel_web_chat response: {turn1_result}")
        })
        .to_string();
    signal_accepted_web_chat_request_id(request_tx, turn1_result);
    let sse_event_1 = sse_task_1.await.expect("sse task 1 join should succeed");
    assert_eq!(
        sse_event_1.get("event").and_then(Value::as_str),
        Some("chat_done"),
        "turn1 should complete successfully: {sse_event_1}"
    );

    let old_reply_id = format!("agent:{turn1_request_id}");

    let before_list = post_json_rpc(
        &rpc_base,
        9702,
        "openhuman.threads_messages_list",
        json!({ "thread_id": thread_id }),
    )
    .await;
    let before_outer = assert_no_jsonrpc_error(&before_list, "threads_messages_list before regen");
    let before_data = before_outer
        .get("data")
        .expect("data envelope in messages_list response");
    let before_messages = before_data
        .get("messages")
        .and_then(Value::as_array)
        .expect("messages array before regen");
    assert!(
        before_messages
            .iter()
            .any(|m| m.get("id").and_then(Value::as_str) == Some(old_reply_id.as_str())),
        "expected the turn1 reply ({old_reply_id}) in the log before regen: {before_messages:?}"
    );
    let before_count = before_messages.len();

    // --- Regenerate the last turn: no message_id, so it redoes turn1. ---
    let (sse_task_2, request_tx) =
        spawn_ready_terminal_web_chat_event_for_request(&events_url).await;
    let regen = post_json_rpc(
        &rpc_base,
        9703,
        "openhuman.threads_regenerate",
        json!({
            "thread_id": thread_id,
            "client_id": client_id,
        }),
    )
    .await;
    let regen_outer = assert_no_jsonrpc_error(&regen, "threads_regenerate");
    let regen_result = peel_logs_envelope(regen_outer);
    let new_request_id = regen_result
        .get("request_id")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("expected 'request_id' in regenerate response: {regen_outer}"));
    assert!(
        !new_request_id.is_empty(),
        "regenerate must return a non-empty request_id: {regen_outer}"
    );
    assert_ne!(
        new_request_id, turn1_request_id,
        "regenerate must restart under a fresh request_id"
    );
    request_tx
        .send(new_request_id.to_owned())
        .expect("SSE reader should receive regenerated turn request_id");

    let sse_event_2 = sse_task_2.await.expect("sse task 2 join should succeed");
    assert_eq!(
        sse_event_2.get("event").and_then(Value::as_str),
        Some("chat_done"),
        "the regenerated turn should complete successfully: {sse_event_2}"
    );

    // --- Verify the store was actually mutated: the old reply is gone,
    // replaced by the fresh turn's own reply. ---
    let after_list = post_json_rpc(
        &rpc_base,
        9704,
        "openhuman.threads_messages_list",
        json!({ "thread_id": thread_id }),
    )
    .await;
    let after_outer = assert_no_jsonrpc_error(&after_list, "threads_messages_list after regen");
    let after_data = after_outer
        .get("data")
        .expect("data envelope in messages_list response");
    let after_messages = after_data
        .get("messages")
        .and_then(Value::as_array)
        .expect("messages array after regen");

    let new_reply_id = format!("agent:{new_request_id}");
    assert!(
        !after_messages
            .iter()
            .any(|m| m.get("id").and_then(Value::as_str) == Some(old_reply_id.as_str())),
        "old reply ({old_reply_id}) must be truncated from the log: {after_messages:?}"
    );
    assert!(
        after_messages
            .iter()
            .any(|m| m.get("id").and_then(Value::as_str) == Some(new_reply_id.as_str())),
        "expected the fresh turn's reply ({new_reply_id}) in the log: {after_messages:?}"
    );
    assert_eq!(
        after_messages.len(),
        before_count,
        "regenerate should replace the reply in place, not grow the log: before={before_count} \
         after={after_messages:?}"
    );

    api_join.abort();
    rpc_join.abort();
}
