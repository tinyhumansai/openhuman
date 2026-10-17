use std::sync::Arc;

use super::*;
use crate::core::runtime::{CoreContext, DomainSet};

#[tokio::test]
async fn spawned_connection_created_task_keeps_its_dispatch_context() {
    let context = CoreContext::for_test(DomainSet::full(), None);
    let expected = Arc::clone(&context);
    let dispatch = DispatchCtx::new(context);

    spawn_connection_created_task(dispatch, move |_task_ctx| async move {
        let current = CoreContext::current().expect("spawn should enter dispatch context");
        assert!(Arc::ptr_eq(&current, &expected));
    })
    .await
    .expect("connection task should complete");
}
