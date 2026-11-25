use super::*;

/// Sanity-check: a parent agent delegating to a sub-agent must complete
/// without panicking, even on a worker thread with a tight stack — this
/// is the same recursion shape that crashed the
/// `chat-harness-subagent` Playwright lane in production with
/// `thread 'tokio-rt-worker' has overflowed its stack, fatal runtime
/// error: stack overflow`.
///
/// The former `chat-harness-subagent.spec.ts` browser lane was intended
/// to exercise real orchestrator → researcher dispatch, but it remained
/// skipped under issue #6389 and was not active passing coverage. The
/// constrained-worker-stack smoke here still checks that this scripted
/// dispatch path remains poll-bounded after refactors; its smaller
/// per-frame state does not cleanly bracket boxed-vs-unboxed with one
/// stack size. See `subagent_host/ops.rs` `Box::pin` callsites for the
/// structural fix.
#[test]
fn nested_subagent_dispatch_runs_on_a_constrained_worker_stack() {
    use async_trait::async_trait;
    use std::sync::Arc;

    struct RecursiveDelegateTool {
        inner_def: AgentDefinition,
        /// The explicit parent the nested run is spawned under (filled once the
        /// parent snapshot, which itself holds this tool, exists).
        parent: Arc<std::sync::OnceLock<ParentExecutionContext>>,
    }

    #[async_trait]
    impl Tool for RecursiveDelegateTool {
        fn name(&self) -> &str {
            "delegate_inner"
        }
        fn description(&self) -> &str {
            "Dispatches a nested sub-agent — reproduces the recursive engine poll."
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type":"object","properties":{}})
        }
        fn permission_level(&self) -> PermissionLevel {
            PermissionLevel::Execute
        }
        async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
            let parent = self
                .parent
                .get()
                .cloned()
                .expect("parent set before the run");
            let outcome = run_host_from_test_options(
                &self.inner_def,
                "inner go",
                with_parent(parent, SubagentRunOptions::default()),
            )
            .await
            .map_err(|e| anyhow::anyhow!("nested child execution failed: {e}"))?;
            Ok(ToolResult::success(outcome.output))
        }
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .thread_stack_size(1024 * 1024)
        .enable_all()
        .build()
        .expect("build constrained-stack tokio runtime");

    let outcome = runtime.block_on(async {
        // Three scripted responses, shared by outer + inner runs
        // (providers are Arc-cloned, so both pull from the same queue):
        //   [0] outer round 1: call `delegate_inner`
        //   [1] inner round 1: return final text
        //   [2] outer round 2: return final text using the tool result
        let provider = ScriptedProvider::new(vec![
            tool_response("delegate_inner", "{}"),
            text_response("inner-final"),
            text_response("outer-final: inner-final"),
        ]);

        let inner_def = make_def_named_tools(&[]);
        let parent_slot = Arc::new(std::sync::OnceLock::new());
        let delegate_tool: Box<dyn Tool> = Box::new(RecursiveDelegateTool {
            inner_def,
            parent: parent_slot.clone(),
        });
        let parent = make_parent(
            Arc::clone(&(provider.clone() as Arc<dyn ChatModel<()>>)),
            vec![delegate_tool],
        );
        let _ = parent_slot.set(parent.clone());
        let outer_def = make_def_named_tools(&["delegate_inner"]);

        (async {
            run_host_from_test_options(
                &outer_def,
                "outer go",
                with_parent(parent.clone(), SubagentRunOptions::default()),
            )
            .await
        })
        .await
    });

    let outcome = outcome.expect(
        "nested child execution must complete on a 1 MiB worker stack — \
         a stack overflow here means the recursion boundary in \
         `run_typed_mode` regressed (see the `Box::pin` callsites around \
         `run_typed_mode` and the child's tinyagents drive future).",
    );
    assert!(
        outcome.output.contains("inner-final"),
        "outer should fold the inner sub-agent's result into its final \
         answer, got: {}",
        outcome.output
    );
}
