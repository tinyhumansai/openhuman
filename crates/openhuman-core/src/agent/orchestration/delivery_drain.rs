//! The ownership fence applied to an asynchronous completion delivery.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use crate::core::runtime::DispatchCtx;
use crate::storage::fence::LeaseFence;
use tinyagents_tasks::DeliveryFence;

/// The profile lease a delivery drain runs under, or no fence for local work.
#[derive(Clone, Debug, Default)]
pub(crate) struct DrainFence(Option<Arc<LeaseFence>>);

impl DrainFence {
    /// No fence: the drain always runs.
    pub(crate) fn none() -> Self {
        Self(None)
    }

    /// The fence of a profile's grant.
    pub(crate) fn of(fence: Arc<LeaseFence>) -> Self {
        Self(Some(fence))
    }

    /// The fence of the profile the calling task serves, when this node hosts
    /// it. `None` outside SaaS or outside a profile.
    pub(crate) fn current(ctx: &DispatchCtx) -> Self {
        Self(crate::profiles::host::current(ctx).map(|profile| Arc::clone(profile.lease_fence())))
    }

    /// Whether this node may still act for the profile.
    pub(crate) fn admits(&self) -> bool {
        self.0.as_ref().is_none_or(|fence| {
            fence
                .check_local(crate::storage::fence::system_now_ms())
                .is_ok()
        })
    }

    /// Sleep for `delay`, cut short when the fence latches. Returns whether
    /// the drain may continue.
    pub(crate) async fn sleep(&self, delay: Duration) -> bool {
        match &self.0 {
            None => {
                tokio::time::sleep(delay).await;
                true
            }
            Some(fence) => {
                tokio::select! {
                    () = tokio::time::sleep(delay) => self.admits(),
                    () = fence.fenced() => false,
                }
            }
        }
    }
}

impl DeliveryFence for DrainFence {
    fn admitted(&self) -> bool {
        self.admits()
    }

    fn lost(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        match &self.0 {
            Some(fence) => Box::pin(async move { fence.fenced().await }),
            None => Box::pin(std::future::pending()),
        }
    }
}
