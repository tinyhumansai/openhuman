//! Per-turn resolved provider/model metadata.
//!
//! Provider wrappers that translate model aliases or perform fallbacks record
//! the concrete route that actually handled the latest successful provider
//! call. The agent bus reads this after the turn so channel audit events can
//! persist the resolved provider/model instead of the caller's requested route.

use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedProviderRoute {
    pub provider: String,
    pub model: String,
}

type RouteSlot = Arc<Mutex<Option<ResolvedProviderRoute>>>;

tokio::task_local! {
    static RESOLVED_PROVIDER_ROUTE: RouteSlot;
}

pub async fn with_resolved_provider_route_scope<F>(future: F) -> F::Output
where
    F: std::future::Future,
{
    tracing::trace!("[provider] resolved-route scope enter");
    let out = RESOLVED_PROVIDER_ROUTE
        .scope(Arc::new(Mutex::new(None)), Box::pin(future))
        .await;
    tracing::trace!("[provider] resolved-route scope exit");
    out
}

pub fn record_resolved_provider_route(provider: impl Into<String>, model: impl Into<String>) {
    let provider = provider.into();
    let model = model.into();
    let route = ResolvedProviderRoute {
        provider: provider.clone(),
        model: model.clone(),
    };
    let wrote = RESOLVED_PROVIDER_ROUTE
        .try_with(|slot| {
            *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(route);
        })
        .is_ok();
    tracing::debug!(
        provider = %provider,
        model = %model,
        wrote,
        "[provider] resolved-route recorded"
    );
}

pub fn current_resolved_provider_route() -> Option<ResolvedProviderRoute> {
    let route = RESOLVED_PROVIDER_ROUTE
        .try_with(|slot| slot.lock().unwrap_or_else(|e| e.into_inner()).clone())
        .ok()
        .flatten();
    tracing::trace!(present = route.is_some(), "[provider] resolved-route read");
    route
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn resolved_provider_route_scopes_and_clears() {
        assert!(current_resolved_provider_route().is_none());

        let observed = with_resolved_provider_route_scope(async {
            record_resolved_provider_route("provider-a", "model-a");
            current_resolved_provider_route()
        })
        .await;

        assert_eq!(
            observed,
            Some(ResolvedProviderRoute {
                provider: "provider-a".into(),
                model: "model-a".into(),
            })
        );
        assert!(current_resolved_provider_route().is_none());
    }
}
