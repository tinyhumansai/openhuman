use super::*;
use std::sync::Mutex;
use tinyjuice_bus::repl::ReplOutput;
use tinytools::ToolCallOptions;

/// Records which core context the OpenHuman adapter forwards to module calls.
#[derive(Default)]
struct CoreRecordingSource(Mutex<Vec<Option<Arc<crate::core::runtime::CoreContext>>>>);

#[async_trait]
impl tinyjuice_host::repl_tools::QuerySource for CoreRecordingSource {
    async fn query(
        &self,
        context: Option<&dyn tinytools::ToolRunContext>,
        _: QueryRequest,
    ) -> Result<QueryResponse, String> {
        let ctx = crate::agent::tinyagents::host::run_context::dispatch_ctx_for_tool(context);
        self.0.lock().unwrap().push(ctx.core().cloned());
        Ok(Ok(ReplOutput::Text { text: "ok".into() }))
    }
}

/// Each tool call uses its own run context rather than a concurrent ambient one.
#[tokio::test]
async fn queries_run_for_the_tool_runs_context_not_the_ambient_one() {
    use crate::agent::tinyagents::host::OpenHumanRunContext;
    use crate::core::runtime::{CoreContext, DomainSet};

    let context_for = |core: Arc<CoreContext>| {
        let run = OpenHumanRunContext::new()
            .with_core(Some(core))
            .into_tinyagents(tinyagents_harness::context::RunConfig::new("juice-carrier"));
        tinyagents_harness::tool::ToolExecutionContext::from_run_context(
            &run,
            tinyagents_harness::ids::CallId::new("juice-carrier-call"),
        )
    };
    let (a, b, ambient) = (
        CoreContext::for_test(DomainSet::full(), None),
        CoreContext::for_test(DomainSet::full(), None),
        CoreContext::for_test(DomainSet::full(), None),
    );
    let source = Arc::new(CoreRecordingSource::default());
    let tools = repl_tools_with(source.clone(), ReplLimits::default());
    let args = serde_json::json!({"handle":"⟦tj:abc123⟧", "query":"needle"});

    CoreContext::scope(ambient.clone(), async {
        for core in [&a, &b] {
            let context = context_for(core.clone());
            tools[0]
                .execute_with_context(args.clone(), ToolCallOptions::default(), Some(&context))
                .await
                .unwrap();
        }
    })
    .await;

    let seen = source.0.lock().unwrap();
    assert!(Arc::ptr_eq(seen[0].as_ref().expect("bound"), &a));
    assert!(Arc::ptr_eq(seen[1].as_ref().expect("bound"), &b));
    assert!(!Arc::ptr_eq(seen[0].as_ref().unwrap(), &ambient));
}
