//! The artifact publisher slot: where an outside crate plugs in the service
//! that shares an artifact version and returns a link.
//!
//! `tiny-artifacts` defines the [`ArtifactPublisher`] port; the core builds
//! its artifact tools and the preview panel's Share action over whichever
//! publisher a registered factory returns for the current configuration. A
//! factory answers `None` when it cannot publish (no backend configured, not
//! signed in), and the core then offers no sharing.
//!
//! The registry is generic: it knows nothing about where a publisher uploads.
//! With nothing registered the core offers no sharing, unchanged.

use std::sync::Arc;

use futures_util::future::BoxFuture;
use tiny_artifacts::{ArtifactPublisher, FileStore};

use crate::config::Config;
use crate::core::slots::{Extensions, HookSlot};

/// Binds a publisher to the file store selected for one artifact service.
pub type ArtifactPublisherBinder =
    Arc<dyn Fn(Arc<dyn FileStore>) -> Arc<dyn ArtifactPublisher> + Send + Sync>;

/// A registered publisher factory.
pub type ArtifactPublisherFactory =
    Arc<dyn for<'a> Fn(&'a Config) -> BoxFuture<'a, Option<ArtifactPublisherBinder>> + Send + Sync>;

static PUBLISHERS: HookSlot<ArtifactPublisherFactory> = HookSlot::new("artifacts::publisher");

/// Registers the publisher factory `id` for the whole process. The first
/// factory (registration order) that returns a publisher serves requests.
pub fn register_artifact_publisher<F>(id: impl Into<String>, factory: F)
where
    F: for<'a> Fn(&'a Config) -> BoxFuture<'a, Option<ArtifactPublisherBinder>>
        + Send
        + Sync
        + 'static,
{
    PUBLISHERS.register(id, Arc::new(factory));
}

/// Registers the publisher factory `id` for the context built from `extensions`.
pub fn register_artifact_publisher_in<F>(extensions: &Extensions, id: impl Into<String>, factory: F)
where
    F: for<'a> Fn(&'a Config) -> BoxFuture<'a, Option<ArtifactPublisherBinder>>
        + Send
        + Sync
        + 'static,
{
    PUBLISHERS.register_in(extensions, id, Arc::new(factory));
}

/// Removes the process-wide publisher factory `id`.
pub fn unregister_artifact_publisher(id: &str) -> bool {
    PUBLISHERS.unregister(id)
}

/// The publisher for `config`, or `None` when no registered factory offers one.
pub async fn artifact_publisher(config: &Config) -> Option<ArtifactPublisherBinder> {
    for (id, factory) in PUBLISHERS.entries_context_first() {
        if let Some(publisher) = factory(config).await {
            log::debug!("[artifacts] publisher `{id}` serves sharing");
            return Some(publisher);
        }
    }
    None
}

/// Selects a publisher using the hook registrations of the supplied call
/// context, then falls back to process registrations as usual.
pub async fn artifact_publisher_in(
    config: &Config,
    ctx: &crate::core::runtime::DispatchCtx,
) -> Option<ArtifactPublisherBinder> {
    let context = ctx.resolved()?;
    crate::core::runtime::CoreContext::scope(context, artifact_publisher(config)).await
}

#[cfg(test)]
#[path = "publisher_slot_tests.rs"]
mod tests;
