use anyhow::Result;
use async_trait::async_trait;
use openhuman_core::agent::hooks::{fire_hooks, PostTurnHook, ToolCallRecord, TurnContext};
use parking_lot::Mutex;
use std::sync::Arc;
use tokio::sync::Notify;

fn sample_turn() -> TurnContext {
    TurnContext {
        user_message: "hello".into(),
        assistant_response: "world".into(),
        tool_calls: vec![ToolCallRecord {
            name: "shell".into(),
            arguments: serde_json::json!({}),
            success: true,
            output_summary: "ok".into(),
            duration_ms: 10,
        }],
        turn_duration_ms: 15,
        session_id: Some("s1".into()),
        agent_id: None,
        entrypoint: None,
        iteration_count: 1,
    }
}

struct RecordingHook {
    name: &'static str,
    calls: Arc<Mutex<Vec<String>>>,
    notify: Arc<Notify>,
    fail: bool,
}

#[async_trait]
impl PostTurnHook for RecordingHook {
    fn name(&self) -> &str {
        self.name
    }

    async fn on_turn_complete(&self, ctx: &TurnContext) -> Result<()> {
        self.calls
            .lock()
            .push(format!("{}:{}", self.name, ctx.user_message));
        self.notify.notify_waiters();
        if self.fail {
            anyhow::bail!("hook failed");
        }
        Ok(())
    }
}

// The legacy `InterruptFence` / `check_interrupt` surface was removed in #4249
// (user-driven cancellation is now the tinyagents steering/cancellation channel),
// so the public-API tests that exercised it are gone with it.

#[tokio::test]
async fn fire_hooks_dispatches_all_hooks_even_when_one_fails() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let notify = Arc::new(Notify::new());
    let hooks: Vec<Arc<dyn PostTurnHook>> = vec![
        Arc::new(RecordingHook {
            name: "ok",
            calls: Arc::clone(&calls),
            notify: Arc::clone(&notify),
            fail: false,
        }),
        Arc::new(RecordingHook {
            name: "fail",
            calls: Arc::clone(&calls),
            notify: Arc::clone(&notify),
            fail: true,
        }),
    ];

    fire_hooks(
        &hooks,
        &openhuman_core::core::runtime::DispatchCtx::unbound(),
        sample_turn(),
    );

    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if calls.lock().len() == 2 {
                break;
            }
            notify.notified().await;
        }
    })
    .await
    .expect("hooks should complete");

    let calls = calls.lock().clone();
    assert!(calls.contains(&"ok:hello".into()));
    assert!(calls.contains(&"fail:hello".into()));
}
