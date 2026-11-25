use super::*;
use crate::security::approval::engine::{ApprovalEngine, CallOrigin, Checked, EngineError};
use std::sync::Arc;

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn the_module_decides_what_each_origin_class_is_allowed() {
    let world = world().await;
    let shown = Arc::new(Shown::default());
    for origin in [CallOrigin::LocalOperator, CallOrigin::TrustedAutomation] {
        let mut call = input("gmail_send", &shown);
        call.origin = origin;
        assert!(
            matches!(
                world.engine.check(call).await.unwrap(),
                Checked::Approved { .. }
            ),
            "{origin:?} is an audited automatic approval"
        );
    }
    assert!(
        shown.deliveries.lock().unwrap().is_empty(),
        "nobody was asked"
    );

    let mut cron = input("gmail_send", &shown);
    cron.origin = CallOrigin::Cron;
    assert_eq!(
        world.engine.check(cron).await.unwrap_err(),
        EngineError::Refused
    );

    let mut unlabelled = input("gmail_send", &shown);
    unlabelled.origin = CallOrigin::Unlabelled;
    assert_eq!(
        world.engine.check(unlabelled).await.unwrap_err(),
        EngineError::Refused
    );
    let mut granted = input("gmail_send", &shown);
    granted.origin = CallOrigin::Unlabelled;
    granted.legacy_tool_grants = vec!["gmail_send".into()];
    assert!(matches!(
        world.engine.check(granted).await.unwrap(),
        Checked::Approved { .. }
    ));
    assert!(shown.deliveries.lock().unwrap().is_empty());
}
