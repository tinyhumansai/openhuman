use super::tests::Probe;
use super::*;
use std::sync::Mutex;

#[tokio::test]
async fn the_wrapped_model_keeps_its_turns_context_while_another_is_active() {
    use crate::core::runtime::{CoreContext, DispatchCtx, DomainSet};
    let turn = CoreContext::for_test(DomainSet::full(), None);
    let other = CoreContext::for_test(DomainSet::full(), None);
    let concrete = AttachmentModel {
        inner: Arc::new(Probe::default()),
        config: Arc::new(Config::default()),
        core: DispatchCtx::new(Arc::clone(&turn)).downgrade(),
        profile: ModelProfile::default(),
        fallback_cache: Mutex::new(Default::default()),
    };
    let seen = CoreContext::scope(Arc::clone(&other), async { concrete.dispatch_ctx() }).await;
    assert!(Arc::ptr_eq(seen.core().expect("bound"), &turn));
    assert!(!Arc::ptr_eq(seen.core().expect("bound"), &other));
}
