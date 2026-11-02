//! The process-wide module runtime and the host callbacks it serves.
//!
//! The runtime itself (a dedicated thread with its own broker, connection and
//! loader) is `tinybus::module::ModuleRuntime`; this file only decides when it
//! starts and which host objects are served on its bus before any module can
//! load.
//!
//! # Why modules get their own bus
//!
//! The core already runs a bus — `core::bus::BUS`, a `OnceBus<DomainEvent>` — but
//! it cannot be reused here. `OnceBus::init_in_process` constructs its `Broker`
//! internally and never hands it out, and the module host needs a `Broker` to
//! attach loaded modules to. So modules run on a second in-process broker, and a
//! module on it can serve requests but cannot publish a `DomainEvent`.
//!
//! # What loading a module means
//!
//! `dlopen` runs code before anything can inspect it. tinybus's ABI descriptor,
//! manifest and digest gates decide whether a module is *admitted*, not whether
//! it is *safe*: once loaded it can read and write this process's memory, and a
//! native fault in it takes the core down. tinybus also never unloads a library,
//! so a module that fails is failed until restart, which is why the resolver
//! caches failures instead of retrying (see [`super::ops`]).
//!
//! Admission is deliberately permissive, tinybus's default. Strict mode also
//! refuses a module whose rustc version string differs from the host's, and
//! released artifacts are built by CI on whatever toolchain that runner had, so
//! strict mode refused the real published artifact outright. Everything that
//! protects the address space is still enforced: the ABI revision, the
//! descriptor layout, the target triple, pointer width, endianness, feature bits
//! and the refusal of a `panic=abort` module.
//!
//! Everything here is created once and lives for the process. There is no
//! shutdown path because there is nothing a shutdown could reclaim.

use std::sync::OnceLock;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::Serialize;
use tinybus::module::{ModuleRuntime, RuntimeNames, RuntimeOptions, SearchPathConfig};

/// The module bus, built once on first use.
static RUNTIME: OnceLock<Result<ModuleRuntime, String>> = OnceLock::new();

/// Host callbacks installed on the runtime's bus, once.
static CALLBACKS: tokio::sync::OnceCell<Result<(), String>> = tokio::sync::OnceCell::const_new();

/// The process-wide module runtime, standing it up on first use.
///
/// # Errors
///
/// Returns an error if the broker's in-memory transport cannot be connected, or
/// a host callback cannot be served. The outcome is cached: a runtime that
/// failed to start does not start on a later call.
pub async fn runtime() -> tinybus::Result<&'static ModuleRuntime> {
    let runtime = runtime_now()?;
    CALLBACKS
        .get_or_init(|| async {
            // On the module runtime, not the caller's: the callbacks serve
            // objects that must outlive whichever runtime asked first.
            match runtime.spawn(install_callbacks(runtime)).await {
                Ok(Ok(())) => Ok(()),
                Ok(Err(error)) => Err(error.to_string()),
                Err(error) => Err(format!("the module callbacks did not finish: {error}")),
            }
        })
        .await
        .as_ref()
        .map_err(|error| tinybus::Error::Transport(error.clone()))?;
    Ok(runtime)
}

/// Schedule host lifecycle cleanup without loading a module or installing callbacks.
pub(crate) fn spawn_cleanup<F>(future: F) -> tinybus::Result<tokio::task::JoinHandle<F::Output>>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    Ok(runtime_now()?.spawn(future))
}

fn runtime_now() -> tinybus::Result<&'static ModuleRuntime> {
    let started = RUNTIME.get_or_init(|| {
        let names = RuntimeNames {
            thread: "openhuman-module-bus".to_string(),
            worker: "openhuman-module-worker".to_string(),
        };
        let runtime = ModuleRuntime::start_with_options(
            &names,
            None,
            RuntimeOptions {
                thread_stack_size: Some(crate::core::runtime::AGENT_WORKER_STACK_BYTES),
            },
        )
        .map_err(|error| error.to_string())?;
        runtime
            .host()
            .set_search_path_config(Some(SearchPathConfig::new(
                "OPENHUMAN_MODULE_PATH",
                "openhuman/modules",
            )));
        Ok(runtime)
    });
    let runtime = started
        .as_ref()
        .map_err(|error| tinybus::Error::Transport(error.clone()))?;
    Ok(runtime)
}

/// Serve the host objects modules call back into, before any module loads.
async fn install_callbacks(runtime: &'static ModuleRuntime) -> tinybus::Result<()> {
    let connection = runtime.connection();

    #[cfg(feature = "security-module")]
    super::security::callbacks::install(connection).await?;
    #[cfg(all(feature = "security-module", feature = "modules"))]
    super::browser_egress_host::install(connection).await?;

    // The summary callback exists for the module alone; with the component off
    // the module never loads, so nothing would call it.
    let host = crate::core::runtime::DispatchCtx::host_default();
    if host.components().juice {
        super::tokenjuice_host::install(connection).await?;
    } else {
        tracing::debug!("[modules] juice component off; MlHost not served");
    }
    super::site_memory_host::install(connection).await?;
    super::storage_host::install(connection, runtime.host()).await?;
    #[cfg(feature = "web3")]
    super::wallet_host::install(connection).await?;
    // Deliver / ClassifySalience for the inbound event queues; with the
    // component off the module never loads, so nothing would call them.
    #[cfg(feature = "inbound-queues")]
    {
        if host.components().eventqueues {
            crate::inbound::host::install(connection).await?;
        } else {
            tracing::debug!("[modules] eventqueues component off; inbound Host not served");
        }
    }
    Ok(())
}

/// Whether the module runtime has been stood up.
///
/// Lets status reporting answer without starting a broker as a side effect of
/// being asked a question.
#[must_use]
pub fn is_started() -> bool {
    RUNTIME.get().is_some_and(Result::is_ok)
}

/// Returns the current runtime only when it was already started and its host
/// callbacks are serving. Unlike [`runtime`], this accessor never starts a
/// runtime or installs callbacks.
pub(crate) fn existing_runtime() -> Option<&'static ModuleRuntime> {
    let runtime = RUNTIME.get()?.as_ref().ok()?;
    CALLBACKS.get()?.as_ref().ok()?;
    Some(runtime)
}

/// A safe, sanitized result for a call through the already-serving module bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExistingModuleCallError {
    Unavailable,
    Failed,
}

/// Call a read-only capability on a module that is already ready. This never
/// starts the module runtime, loads a module, or retries through a ModuleClient.
pub(crate) async fn call_if_ready<Request, Reply>(
    id: &str,
    member: &str,
    request: Request,
    timeout: Duration,
) -> Result<Option<Reply>, ExistingModuleCallError>
where
    Request: Serialize + Send,
    Reply: DeserializeOwned,
{
    if super::ops::state_of(id) != super::types::ModuleState::Ready {
        return Ok(None);
    }
    let Some(runtime) = existing_runtime() else {
        return Ok(None);
    };
    let record = super::registry::find(id).ok_or(ExistingModuleCallError::Unavailable)?;
    let proxy = runtime
        .proxy(record.bus_name, record.object_path)
        .map_err(|_| ExistingModuleCallError::Unavailable)?
        .with_timeout(timeout);
    match tokio::time::timeout(
        timeout,
        proxy.call_confidential::<Reply>(member, (request,)),
    )
    .await
    {
        Ok(Ok(reply)) => Ok(Some(reply)),
        Ok(Err(_)) | Err(_) => Err(ExistingModuleCallError::Failed),
    }
}

/// Whether `id` is admitted and currently serving on an already-started
/// module runtime. This is a read-only snapshot; it never starts the runtime.
#[must_use]
pub fn is_module_serving(id: &str) -> bool {
    let Some(record) = crate::modules::registry::find(id) else {
        return false;
    };
    let Some(Ok(runtime)) = RUNTIME.get() else {
        return false;
    };
    runtime.host().list().into_iter().any(|module| {
        module.manifest.bus_name.as_str() == record.bus_name
            && module.manifest.object_path.as_str() == record.object_path
            && module.enabled
            && matches!(
                module.state,
                tinybus::module::ModuleState::Ready | tinybus::module::ModuleState::Serving
            )
    })
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
