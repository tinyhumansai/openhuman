
fn handle_turn_state_clear(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<ClearTurnStateRequest>(params)?;
        to_json(ops::turn_state_clear(p).await?)
    })
}

#[derive(serde::Deserialize)]
struct TaskBoardGetParams {
    thread_id: String,
}

#[derive(serde::Deserialize)]
struct TaskBoardPutParams {
    thread_id: String,
    cards: Vec<TaskBoardCard>,
}

fn handle_task_board_get(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<TaskBoardGetParams>(params)?;
        let thread_id = p.thread_id.trim().to_string();
        tracing::debug!(thread_id = %thread_id, "[rpc][task_board] get entry");
        let config = crate::openhuman::config::Config::load_or_init()
            .await
            .map_err(|e| {
                tracing::debug!(
                    thread_id = %thread_id,
                    error = %e,
                    "[rpc][task_board] get load_config_error"
                );
                format!("load config: {e}")
            })?;
        tracing::trace!(thread_id = %thread_id, "[rpc][task_board] get loading_board");
        let board = crate::openhuman::agent::task_board::board_for_thread(
            &config.workspace_dir,
            &thread_id,
        )
        .await
        .map_err(|e| {
            tracing::debug!(
                thread_id = %thread_id,
                error = %e,
                "[rpc][task_board] get board_error"
            );
            e
        })?;
        tracing::debug!(
            thread_id = %thread_id,
            card_count = board.cards.len(),
            "[rpc][task_board] get exit"
        );
        Ok(serde_json::json!({ "taskBoard": board }))
    })
}

fn handle_task_board_put(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<TaskBoardPutParams>(params)?;
        let thread_id = p.thread_id.trim().to_string();
        tracing::debug!(
            thread_id = %thread_id,
            card_count = p.cards.len(),
            "[rpc][task_board] put entry"
        );
        let config = crate::openhuman::config::Config::load_or_init()
            .await
            .map_err(|e| {
                tracing::debug!(
                    thread_id = %thread_id,
                    error = %e,
                    "[rpc][task_board] put load_config_error"
                );
                format!("load config: {e}")
            })?;
        let board = TaskBoard {
            thread_id: thread_id.clone(),
            cards: p.cards,
            updated_at: chrono::Utc::now().to_rfc3339(),
        };
        let saved = TaskBoardStore::new(config.workspace_dir)
            .put(board)
            .await
            .map_err(|e| {
                tracing::debug!(
                    thread_id = %thread_id,
                    error = %e,
                    "[rpc][task_board] put store_error"
                );
                e
            })?;
        tracing::debug!(
            thread_id = %thread_id,
            card_count = saved.cards.len(),
            "[rpc][task_board] put exit"
        );
        Ok(serde_json::json!({ "taskBoard": saved }))
    })
}

fn handle_token_usage(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<ops::ThreadTokenUsageRequest>(params)?;
        to_json(ops::token_usage(p).await?)
    })
}

fn handle_transcript_get(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<ops::TranscriptGetRequest>(params)?;
        to_json(ops::transcript_get(p).await?)
    })
}

// ── Helpers ──────────────────────────────────────────────────────────

fn parse<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn to_json<T: serde::Serialize>(outcome: crate::rpc::RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}
