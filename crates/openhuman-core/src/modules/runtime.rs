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

use tinybus::module::{ModuleRuntime, RuntimeNames};

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
    let started = RUNTIME.get_or_init(|| {
        let names = RuntimeNames {
            thread: "openhuman-module-bus".to_string(),
            worker: "openhuman-module-worker".to_string(),
        };
        ModuleRuntime::start(&names, None).map_err(|error| error.to_string())
    });
    let runtime = started
        .as_ref()
        .map_err(|error| tinybus::Error::Transport(error.clone()))?;
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

/// Serve the host objects modules call back into, before any module loads.
async fn install_callbacks(runtime: &'static ModuleRuntime) -> tinybus::Result<()> {
    let connection = runtime.connection();

    // The summary callback exists for the module alone; with the component off
    // the module never loads, so nothing would call it.
    let juice_on = crate::core::runtime::context::CoreContext::current()
        .is_none_or(|ctx| ctx.components().juice);
    if juice_on {
        super::tokenjuice_host::install(connection).await?;
    } else {
        tracing::debug!("[modules] juice component off; MlHost not served");
    }
    #[cfg(feature = "security-module")]
    super::security_host::install(connection).await?;
    super::site_memory_host::install(connection).await?;
    super::storage_host::install(connection, runtime.host()).await?;
    #[cfg(feature = "tinymemes")]
    {
        let memes_on = crate::core::runtime::context::CoreContext::current()
            .is_none_or(|ctx| ctx.components().memes);
        if memes_on {
            crate::tinymemes::service::install(connection).await?;
        } else {
            tracing::debug!("[modules] memes component off; Host callbacks not served");
        }
    }
    #[cfg(feature = "web3")]
    super::wallet_host::install(connection).await?;
    Ok(())
}

/// The running module runtime, without starting it.
pub(crate) fn started() -> Option<&'static ModuleRuntime> {
    RUNTIME.get().and_then(|r| r.as_ref().ok())
}

/// Whether the module runtime has been stood up.
///
/// Lets status reporting answer without starting a broker as a side effect of
/// being asked a question.
#[must_use]
pub fn is_started() -> bool {
    RUNTIME.get().is_some_and(Result::is_ok)
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
