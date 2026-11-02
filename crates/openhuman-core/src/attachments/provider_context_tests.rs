use super::tests::Probe;
use super::*;
use std::sync::Mutex;

#[tokio::test]
async fn the_wrapped_model_keeps_its_turns_context_while_another_is_active() {
    use crate::core::runtime::{CoreContext, DispatchCtx, DomainSet};
    let turn = CoreContext::for_test(DomainSet::full(), None);
    let other = CoreContext::for_test(DomainSet::full(), None);
    let concrete = AttachmentModel {
        config: Arc::new(Config::default()),
        core: DispatchCtx::new(Arc::clone(&turn)).downgrade(),
        fallback_cache: Mutex::new(Default::default()),
        controls: None,
    };
    let seen = CoreContext::scope(Arc::clone(&other), async { concrete.dispatch_ctx() }).await;
    assert!(Arc::ptr_eq(seen.core().expect("bound"), &turn));
    assert!(!Arc::ptr_eq(seen.core().expect("bound"), &other));
}

#[test]
fn owner_media_wrapper_does_not_keep_a_turn_context_alive() {
    use crate::core::runtime::{CoreContext, DispatchCtx, DomainSet};
    let context = CoreContext::for_test(DomainSet::full(), None);
    let weak = Arc::downgrade(&context);
    let dispatch = DispatchCtx::new(Arc::clone(&context));
    let wrapper = owner::wrap(
        &dispatch,
        Arc::new(Probe::default()),
        Arc::new(Config::default()),
        Some(ModelProfile::default()),
        None,
    );
    drop(dispatch);
    drop(context);
    assert!(
        weak.upgrade().is_none(),
        "the provider wrapper retains only weak turn context"
    );
    drop(wrapper);
}
