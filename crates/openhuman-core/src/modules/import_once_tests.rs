use super::*;

#[derive(Debug)]
struct Done(bool);

impl Settled for Done {
    fn settled(&self) -> bool {
        self.0
    }
}

#[tokio::test]
async fn a_concurrent_caller_waits_for_the_running_import() {
    let ctx = crate::core::runtime::DispatchCtx::host_default();
    let dir = std::path::PathBuf::from("/import-once/concurrent");
    let finished = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = finished.clone();
    let first = import_once(&ctx, "test-concurrent", MODULE_FOR_TEST, &dir, async move {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(Done(true))
    });
    let flag = finished.clone();
    let second = async {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        import_once(&ctx, "test-concurrent", MODULE_FOR_TEST, &dir, async {
            panic!("a settled import must not run a second time");
            #[allow(unreachable_code)]
            Ok(Done(true))
        })
        .await;
        flag.load(std::sync::atomic::Ordering::SeqCst)
    };
    let (_, saw_finished) = tokio::join!(first, second);
    assert!(
        saw_finished,
        "second caller returned before the import ended"
    );
}

#[tokio::test]
async fn an_unsettled_outcome_and_a_cancelled_import_are_retried() {
    let ctx = crate::core::runtime::DispatchCtx::host_default();
    let dir = std::path::PathBuf::from("/import-once/retry");
    let runs = std::sync::atomic::AtomicUsize::new(0);
    let count = || runs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    import_once(&ctx, "test-retry", MODULE_FOR_TEST, &dir, async {
        count();
        Ok(Done(false))
    })
    .await;
    let cancelled = tokio::time::timeout(
        std::time::Duration::from_millis(500),
        import_once(&ctx, "test-retry", MODULE_FOR_TEST, &dir, async {
            count();
            std::future::pending::<anyhow::Result<Done>>().await
        }),
    )
    .await;
    assert!(cancelled.is_err());
    import_once(&ctx, "test-retry", MODULE_FOR_TEST, &dir, async {
        count();
        Ok(Done(true))
    })
    .await;
    import_once(&ctx, "test-retry", MODULE_FOR_TEST, &dir, async {
        count();
        Ok(Done(true))
    })
    .await;
    assert_eq!(runs.load(std::sync::atomic::Ordering::SeqCst), 3);
}

#[tokio::test]
async fn the_import_is_keyed_by_the_explicit_contexts_scope_not_the_tasks() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DispatchCtx, DomainSet};
    let profile = |name: &str| {
        DispatchCtx::new(
            CoreContext::for_test(DomainSet::full(), None).derive_with(
                ContextOverlay::new(
                    crate::config::Config::default(),
                    DomainSet::kernel(),
                    Default::default(),
                )
                .profile(name),
            ),
        )
    };
    let dir = std::path::PathBuf::from("/import-once/tenants");
    let runs = std::sync::atomic::AtomicUsize::new(0);
    for ctx in [
        profile("once-alice"),
        profile("once-bob"),
        profile("once-alice"),
    ] {
        import_once(&ctx, "test-tenants", MODULE_FOR_TEST, &dir, async {
            runs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(Done(true))
        })
        .await;
    }
    assert_eq!(
        runs.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "each tenant imports once; a repeat for the same tenant is skipped"
    );
}

const MODULE_FOR_TEST: &str = "tinyjuice";
