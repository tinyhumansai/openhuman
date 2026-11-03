//! Loading a module that takes host callbacks, from whichever source serves it.
//!
//! TinyBus binds host callbacks to a module before its `init` runs, and only
//! on paths that keep the admitted identity: an explicit local file (a `path`
//! override, a source build, or a staged `OPENHUMAN_MODULE_PATH` release) and
//! the pinned release chain itself. The release chain tries the installer
//! bundle, then the release cache, then a download into the cache when
//! `modules.allow_download` is on, and every candidate passes the registry's
//! archive SHA-256 and, where the record pins one, its library SHA-256 before
//! a binding is offered to it.
//!
//! `Resolver::ensure_loaded_with_host_callback_factory` drives both, preparing
//! a fresh binding for each candidate so a refused candidate revokes only its
//! own. `Resolver::ensure_loaded_with_host_callbacks` takes one binding and
//! accepts only a local-file source, so a release-served module (every shipped
//! host without `OPENHUMAN_MODULE_PATH`) could neither be loaded nor
//! re-checked through it. Every callback-bound load goes through [`load`] and
//! every later check through [`confirm`] instead.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

use tinybus::module::{HostCallbackBinding, Resolver};

/// One prepared candidate: its binding, the module configuration naming it,
/// and the host state that must live as long as the binding is retained.
pub(in crate::modules) type Prepared<S> = (Arc<HostCallbackBinding>, serde_json::Value, S);

/// Load `id` with host callbacks, preparing a fresh set with `prepare` for
/// each candidate artifact the resolver tries.
///
/// Returns the binding the admitted module retains and the state prepared
/// with it; the state of refused candidates is dropped.
///
/// # Errors
///
/// The resolver's message when no candidate is admitted, `prepare`'s message
/// when a binding cannot be prepared, or a refusal when the module is already
/// serving with a binding this call did not prepare.
pub(in crate::modules) async fn load<S, F, Fut>(
    resolver: &Resolver,
    id: &str,
    prepare: F,
) -> Result<(Arc<HostCallbackBinding>, S), String>
where
    S: Send + 'static,
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Prepared<S>, String>> + Send + 'static,
{
    let candidates: Arc<Mutex<HashMap<String, S>>> = Arc::new(Mutex::new(HashMap::new()));
    let factory_candidates = Arc::clone(&candidates);
    let prepare = Arc::new(prepare);
    let binding = resolver
        .ensure_loaded_with_host_callback_factory(id, move |_attempt| {
            let candidates = Arc::clone(&factory_candidates);
            let prepare = Arc::clone(&prepare);
            async move {
                let (binding, module_config, state) = prepare().await?;
                candidates
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(binding.unique_name().to_string(), state);
                Ok((binding, module_config))
            }
        })
        .await?;
    let state = candidates
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(binding.unique_name().as_str())
        .ok_or_else(|| {
            format!("module '{id}' is serving with a callback binding this load did not prepare")
        })?;
    Ok((binding, state))
}

/// Confirm `id` is still admitted with `expected`, preparing nothing.
///
/// For a module an earlier [`load`] admitted: the resolver answers from the
/// retained binding without touching any source, so this works whether the
/// module came from a local file or from the release chain.
///
/// # Errors
///
/// The resolver's cached failure, or a refusal when the module is not serving
/// with `expected`.
pub(in crate::modules) async fn confirm(
    resolver: &Resolver,
    id: &str,
    expected: &HostCallbackBinding,
) -> Result<(), String> {
    let module_id = id.to_owned();
    let retained = resolver
        .ensure_loaded_with_host_callback_factory(id, move |_attempt| {
            let id = module_id.clone();
            async move {
                Err(format!(
                    "module '{id}' is not serving with its admitted callback binding"
                ))
            }
        })
        .await?;
    if retained.unique_name() != expected.unique_name() {
        return Err(format!(
            "module '{id}' is serving with a different callback binding"
        ));
    }
    Ok(())
}
