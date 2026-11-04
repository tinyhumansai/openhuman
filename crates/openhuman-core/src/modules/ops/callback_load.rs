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
//! every later check through [`confirm`] instead, each run through
//! [`detached`] so a caller that stops waiting cannot strand the binding's
//! host state.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

use crate::config::Config;
use crate::core::runtime::CoreContext;
use crate::modules::types::ModuleState;
use tinybus::module::{HostCallbackBinding, Resolver};

/// Run a callback load on the module runtime and wait for it.
///
/// TinyBus finishes a resolution it has started even when every caller stops
/// waiting (`ensure_loaded_within` bounds only the wait). A callback load also
/// publishes the admitted binding's host state, a storage endpoint or a
/// consequence host, into a process-wide cache, and that publication must not
/// be cancelled with the caller either: the next caller would find the module
/// serving with a binding nobody holds the state for. Dropping the returned
/// future drops only the wait; `work` runs to completion.
///
/// The task is scheduled before anything is awaited, so a caller that gives
/// up while the module bus is still installing its host callbacks does not
/// lose the load either: the task awaits them itself. It runs under the
/// caller's [`CoreContext`], captured here: TinySecurity's first load prepares
/// its secret scope inside `work`, and that resolves the caller's profile,
/// agent and storage scope from the ambient context.
///
/// # Errors
///
/// `work`'s error, the module bus failing to start, or the task panicking.
pub(in crate::modules) async fn detached<T, Fut>(
    resolver: &Resolver,
    work: Fut,
) -> Result<T, String>
where
    T: Send + 'static,
    Fut: Future<Output = Result<T, String>> + Send + 'static,
{
    let work = CoreContext::propagate(work);
    let resolver = resolver.clone();
    let task = crate::modules::runtime::spawn_on_bus(async move {
        resolver
            .runtime()
            .await
            .map_err(|error| error.to_string())?;
        work.await
    })
    .map_err(|error| error.to_string())?;
    task.await
        .map_err(|error| format!("the module callback load did not finish: {error}"))?
}

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

/// The body [`super::ensure_loaded_with_storage_callback`] runs detached.
pub(super) async fn load_with_storage_callback(
    resolver: &Resolver,
    config: &Config,
    id: &str,
) -> Result<(), String> {
    let mut loads = super::STORAGE_CALLBACK_LOADS.lock().await;
    if let Some((binding, _served)) = loads.get(id) {
        if matches!(super::state_of(id), ModuleState::Ready) {
            // Already serving with this binding: a binding is spent once the
            // module takes it, so re-presenting it would be refused. The module
            // is process-wide, so every later workspace's client finds it
            // already serving.
            return Ok(());
        }
        return confirm(resolver, id, binding).await;
    }

    let runtime = resolver
        .runtime()
        .await
        .map_err(|error| error.to_string())?;
    let config = config.clone();
    let module_id = id.to_owned();
    let (binding, served) = load(resolver, id, move || {
        let config = config.clone();
        let id = module_id.clone();
        async move {
            let (binding, served) = crate::modules::storage_host::prepare_module_callbacks(runtime)
                .await
                .map_err(|error| error.to_string())?;
            let mut module_config = super::module_config(&config, &id).await;
            module_config["storage_callback"] = serde_json::json!(binding.unique_name().as_str());
            Ok((binding, module_config, served))
        }
    })
    .await?;
    loads.insert(id.to_owned(), (binding, served));
    Ok(())
}

#[cfg(feature = "security-module")]
/// The body [`super::ensure_loaded_with_computer_callback`] runs detached.
pub(super) async fn load_with_computer_callback(
    resolver: &Resolver,
    config: &Config,
    id: &str,
) -> Result<(), String> {
    let mut loads = super::COMPUTER_CALLBACK_LOADS.lock().await;
    if let Some(binding) = loads.get(id) {
        return confirm(resolver, id, binding).await;
    }

    let runtime = resolver
        .runtime()
        .await
        .map_err(|error| error.to_string())?;
    let config = config.clone();
    let module_id = id.to_owned();
    let (binding, ()) = load(resolver, id, move || {
        let config = config.clone();
        let id = module_id.clone();
        async move {
            let binding =
                crate::modules::browser_egress_host::prepare_consequence_callback(runtime)
                    .await
                    .map_err(|error| error.to_string())?;
            let module_config =
                super::module_config_with_consequence_callback(&config, &id, &binding).await;
            Ok((binding, module_config, ()))
        }
    })
    .await?;
    loads.insert(id.to_owned(), binding);
    Ok(())
}
