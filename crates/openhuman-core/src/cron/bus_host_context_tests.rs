use super::*;

use super::tests::{config, flow_job};
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::TempDir;

#[tokio::test]
async fn an_expired_bound_callback_fails_closed_under_another_ambient_context() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DispatchCtx, DomainSet};

    let tmp = TempDir::new().unwrap();
    let config = config(&tmp);
    let expired = CoreContext::for_test_with_config(DomainSet::full(), config.clone()).derive_with(
        ContextOverlay::new(
            config.clone(),
            DomainSet::full(),
            crate::tools::toolpacks::ToolGroups::none(),
        )
        .session_agent(format!(
            "expired-callback-{}",
            uuid::Uuid::new_v4().simple()
        )),
    );
    let expired_weak = Arc::downgrade(&expired);
    let dispatch = DispatchCtx::new(Arc::clone(&expired));
    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.action_dir,
    ));
    let binding = Binding::new(&dispatch, &config, security);
    let host = ScheduleHost {
        executor: Arc::new(HostExecutor::new(binding.clone())),
        binding,
    };
    drop(dispatch);
    drop(expired);
    assert!(
        expired_weak.upgrade().is_none(),
        "the bound context is truly expired before the callback"
    );

    let other = CoreContext::for_test_with_config(DomainSet::full(), config.clone()).derive_with(
        ContextOverlay::new(
            config.clone(),
            DomainSet::full(),
            crate::tools::toolpacks::ToolGroups::none(),
        )
        .session_agent("other-callback-scope"),
    );
    let name = format!("expired-callback-{}", uuid::Uuid::new_v4().simple());
    let calls = Arc::new(AtomicUsize::new(0));
    let handler_calls = Arc::clone(&calls);
    let _handler = crate::cron::system_job_handlers::register(
        &name,
        Arc::new(move |_| {
            handler_calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(()) })
        }),
    );
    let job = flow_job(&config, &format!("system:{name}"));
    let result = CoreContext::scope(other, async {
        host.execute(job, "expired-context-run".to_string()).await
    })
    .await;
    assert!(result.is_err(), "an expired bound callback must be refused");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "the callback must not execute under the unrelated ambient context"
    );
}
