//! Task-local carrier for the **current session/sub-agent's user-configured
//! vision capability** so the deep turn engine's image gate can honor a custom
//! (BYOK) model's `model_registry.vision` flag without widening
//! [`crate::openhuman::agent::harness::engine::run_turn_engine`]'s signature.
//!
//! Managed-backend models advertise vision via `Provider::supports_vision()`, so
//! the gate accepts their image turns already. Custom OpenAI-compatible providers
//! report `supports_vision() == false` (a provider endpoint can't know a per-model
//! property), so without this the gate would reject image turns for a model the
//! user explicitly marked vision-capable. This task-local surfaces that per-model
//! flag — computed once at session build (where the full `Config` / `model_registry`
//! and the resolved model id coexist) — to the gate.
//!
//! Mirrors [`super::sandbox_context`]. When unset (CLI / direct invocation / tests
//! that never wrapped the call) [`current_model_vision`] returns `None` and the
//! gate falls back to the provider capability only — strictly additive.

tokio::task_local! {
    /// User-configured vision capability for the currently-executing
    /// session/sub-agent's model. Scoped per turn by the turn loop + subagent
    /// runner. `None` when unset.
    pub static CURRENT_MODEL_VISION: bool;
}

/// Returns the current model's user-configured vision flag, if scope is active.
pub fn current_model_vision() -> Option<bool> {
    CURRENT_MODEL_VISION.try_with(|v| *v).ok()
}

/// Run `future` with `vision` installed as the current model's vision flag.
/// Intended call site is around each `run_turn_engine` invocation.
pub async fn with_current_model_vision<F, R>(vision: bool, future: F) -> R
where
    F: std::future::Future<Output = R>,
{
    CURRENT_MODEL_VISION.scope(vision, future).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn current_model_vision_returns_none_outside_scope() {
        assert_eq!(current_model_vision(), None);
    }

    #[tokio::test]
    async fn with_current_model_vision_installs_value() {
        let observed = with_current_model_vision(true, async { current_model_vision() }).await;
        assert_eq!(observed, Some(true));
    }

    #[tokio::test]
    async fn with_current_model_vision_does_not_leak_across_scopes() {
        with_current_model_vision(true, async {
            assert_eq!(current_model_vision(), Some(true));
        })
        .await;
        assert_eq!(current_model_vision(), None);
    }

    #[tokio::test]
    async fn nested_scope_overrides_outer() {
        with_current_model_vision(false, async {
            assert_eq!(current_model_vision(), Some(false));
            with_current_model_vision(true, async {
                assert_eq!(current_model_vision(), Some(true));
            })
            .await;
            assert_eq!(current_model_vision(), Some(false));
        })
        .await;
    }
}
