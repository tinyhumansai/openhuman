//! The scope a module's host call serves follows the dispatch context passed
//! in, not the calling task's scope.

use super::*;
use crate::core::runtime::{ContextOverlay, CoreContext, DispatchCtx, DomainSet};

fn profile(name: &str) -> DispatchCtx {
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
}

#[tokio::test]
async fn a_module_scope_is_the_explicit_contexts_profile_across_tasks() {
    let (alice, bob) = (profile("scope-alice"), profile("scope-bob"));
    let resolve = |ctx: DispatchCtx| {
        tokio::spawn(async move {
            tokio::task::yield_now().await;
            current_module_scope(&ctx, "tinymcp").map(|scope| scope.as_str().to_owned())
        })
    };
    let (a, b) = tokio::join!(resolve(alice.clone()), resolve(bob));
    let (a, b) = (a.unwrap().unwrap(), b.unwrap().unwrap());
    assert!(a.contains("scope-alice") && !a.contains("scope-bob"), "{a}");
    assert!(b.contains("scope-bob") && !b.contains("scope-alice"), "{b}");
    assert_eq!(
        current_module_scope(&alice, "tinymcp").unwrap().as_str(),
        a,
        "the same context resolves the same scope"
    );
}
