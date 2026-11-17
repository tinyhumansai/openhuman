use super::*;
use crate::core::runtime::agent_scope::test_agent_context;
use crate::core::runtime::{context::CoreContext, DomainSet};

#[tokio::test]
async fn request_journal_runs_stay_with_the_agent_that_registered_them() {
    let root = CoreContext::for_test(DomainSet::full(), None);
    let alpha = test_agent_context(&root, "alpha");
    let beta = test_agent_context(&root, "beta");

    {
        let scope_ctx = crate::core::runtime::DispatchCtx::new(std::sync::Arc::clone(&alpha));

        register_request_journal_run(&scope_ctx, "req-shared", "run-alpha");
    };

    assert_eq!(
        {
            let scope_ctx = crate::core::runtime::DispatchCtx::new(beta);

            take_request_journal_run(&scope_ctx, "req-shared")
        },
        None
    );
    assert_eq!(
        {
            let scope_ctx = crate::core::runtime::DispatchCtx::new(alpha);

            take_request_journal_run(&scope_ctx, "req-shared")
        },
        Some("run-alpha".to_string())
    );
}
