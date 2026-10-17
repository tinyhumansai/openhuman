use super::*;

use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};
use crate::tools::toolpacks::ToolGroups;

fn profile_ctx(root: &std::sync::Arc<CoreContext>, profile: &str) -> DispatchCtx {
    DispatchCtx::new(
        root.derive_with(
            ContextOverlay::new(
                crate::config::Config::default(),
                DomainSet::kernel(),
                ToolGroups::none(),
            )
            .profile(profile),
        ),
    )
}

#[tokio::test]
async fn the_explicit_context_decides_whether_the_caller_acts_for_a_profile() {
    let root = CoreContext::for_test(DomainSet::full(), None);
    let profile = profile_ctx(&root, "u-alice");
    let plain = DispatchCtx::new(root.clone());

    // Each answer holds while the task is ambient under the other tenant,
    // across an await and in a spawned task.
    let for_profile = tokio::spawn(CoreContext::scope(root.clone(), {
        let profile = profile.clone();
        async move {
            tokio::task::yield_now().await;
            acts_for_a_profile(&profile)
        }
    }));
    let for_plain = tokio::spawn(CoreContext::scope(
        profile.core().unwrap().clone(),
        async move {
            tokio::task::yield_now().await;
            acts_for_a_profile(&plain)
        },
    ));
    assert!(for_profile.await.unwrap());
    assert!(!for_plain.await.unwrap());
}
