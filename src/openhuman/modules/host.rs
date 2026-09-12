//! The module host: a broker, a connection, and the loaded-module table.
//!
//! # Why modules get their own bus
//!
//! The core already runs a bus — `core::bus::BUS`, a `OnceBus<DomainEvent>` — but
//! it cannot be reused here. `OnceBus::init_in_process` constructs its `Broker`
//! internally and never hands it out, and `ModuleHost::new` needs a `Broker` to
//! attach loaded modules to. Rather than change tinybus to expose it, modules run
//! on a second in-process broker of their own.
//!
//! That is a real limitation and worth naming: a module on this bus cannot
//! publish a `DomainEvent`, so it can serve requests but cannot participate in
//! the core's event flow. For a codec that is exactly right — a document writer
//! has nothing to say to the subconscious. A module that did need to emit events
//! would need `OnceBus` to share its broker first.
//!
//! # What loading a module means
//!
//! `dlopen` runs code before anything can inspect it. tinybus's ABI descriptor,
//! manifest and digest gates decide whether a module is *admitted*, not whether
//! it is *safe*: once loaded it can read and write this process's memory, and a
//! native fault in it takes the core down. It is first-party code that happens to
//! ship separately. tinybus also never unloads a library, so a module that fails
//! is failed until restart — which is why [`super::ops`] caches failures instead
//! of retrying.
//!
//! Everything here is created once and lives for the process. There is no
//! shutdown path because there is nothing a shutdown could reclaim.
//!
//! The broker lives on a dedicated process-lifetime Tokio runtime. This keeps a
//! module usable across short-lived caller runtimes (notably `#[tokio::test]`)
//! and also prevents an embedding host from accidentally tying module lifetime
//! to an independently managed application task runtime.

use std::sync::Arc;
use std::sync::OnceLock;

use tinybus::broker::Broker;
use tinybus::module::ModuleHost;
use tinybus::transport::memory::MemoryBus;
use tinybus::{Connection, Proxy};

/// The module bus, built once on first use.
static RUNTIME: OnceLock<ModuleRuntime> = OnceLock::new();

/// The broker, the host's own connection to it, and the module loader.
pub struct ModuleRuntime {
    /// The loader. Owns every admitted module for the process lifetime.
    host: ModuleHost,
    /// This process's client connection, used to call into loaded modules.
    connection: Connection,
    /// Handle for work that must spawn module transport tasks with process
    /// lifetime rather than the caller runtime's lifetime.
    handle: tokio::runtime::Handle,
}

impl ModuleRuntime {
    /// The loader.
    #[must_use]
    pub fn host(&self) -> &ModuleHost {
        &self.host
    }

    /// The connection modules are called over.
    #[must_use]
    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    /// Spawn `future` with process lifetime.
    ///
    /// For work that other callers wait on — a module resolution — and that
    /// must therefore outlive whichever caller runtime started it.
    pub fn spawn<F>(&self, future: F) -> tokio::task::JoinHandle<F::Output>
    where
        F: std::future::Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.handle.spawn(future)
    }

    /// Run module admission work on this runtime's blocking pool.
    pub async fn blocking<F>(&self, work: F) -> Result<(), String>
    where
        F: FnOnce() -> Result<(), String> + Send + 'static,
    {
        self.handle
            .spawn_blocking(work)
            .await
            .map_err(|error| format!("the module loader did not finish: {error}"))?
    }

    /// A proxy for one object on a loaded module.
    ///
    /// # Errors
    ///
    /// Returns an error if `bus_name` or `object_path` is not well formed, which
    /// for a registry entry means the table is wrong rather than the module.
    pub fn proxy(&self, bus_name: &str, object_path: &str) -> tinybus::Result<Proxy> {
        self.connection.proxy(bus_name, object_path, bus_name)
    }
}

/// The process-wide module runtime, standing it up on first use.
///
/// # Errors
///
/// Returns an error if the broker's in-memory transport cannot be connected,
/// which in practice means the tokio runtime is shutting down.
///
/// # Panics
///
/// Does not panic: a lost initialisation race reuses the winner's runtime.
pub async fn runtime() -> tinybus::Result<&'static ModuleRuntime> {
    if let Some(existing) = RUNTIME.get() {
        return Ok(existing);
    }

    static START: OnceLock<Result<(), String>> = OnceLock::new();
    let started = START.get_or_init(|| {
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("openhuman-module-bus".to_string())
            .spawn(move || {
                let tokio_runtime = match tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .thread_name("openhuman-module-worker")
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = ready_tx.send(Err(error.to_string()));
                        return;
                    }
                };
                let result = tokio_runtime.block_on(build_runtime());
                match result {
                    Ok(runtime) => {
                        let _ = RUNTIME.set(runtime);
                        let _ = ready_tx.send(Ok(()));
                        tokio_runtime.block_on(std::future::pending::<()>());
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error.to_string()));
                    }
                }
            })
            .map_err(|error| error.to_string())?;
        ready_rx.recv().map_err(|error| error.to_string())?
    });
    started
        .as_ref()
        .map_err(|error| tinybus::Error::Transport(error.clone()))?;
    RUNTIME
        .get()
        .ok_or_else(|| tinybus::Error::Transport("module runtime did not start".to_string()))
}

async fn build_runtime() -> tinybus::Result<ModuleRuntime> {
    let transport = MemoryBus::new();
    let broker = Broker::new();
    // The broker task is deliberately not retained. It lives as long as the
    // process, and holding the handle would only offer an abort that must never
    // be called: a module whose transport disappears faults, and a faulted
    // module cannot be reloaded without a restart.
    broker.spawn(transport.clone());

    // Permissive admission, which is tinybus's default, and the choice is
    // deliberate rather than inherited.
    //
    // Strict mode additionally refuses a module whose rustc version string
    // differs from the host's. That sounds like the safer setting and is the
    // wrong one here: released artifacts are built by CI on whatever toolchain
    // that runner had, and this crate pins its own. Turning strict on was tried
    // first and refused the real published artifact outright — `module
    // libtinydocs_module.so refused: rustc version does not match in strict
    // mode` — which would have meant the feature never worked in the field
    // while every local build looked fine.
    //
    // Everything that protects the address space is still enforced in permissive
    // mode: the ABI revision, the descriptor layout, the target triple, pointer
    // width, endianness, feature bits, and the refusal of a panic=abort module.
    // Only the toolchain string is relaxed, and it is reported rather than
    // ignored. If a rustc mismatch ever does break a module, the failure is a
    // refusal or a fault at load time, not silent corruption.
    let host = ModuleHost::new(broker);
    let connection = Connection::connect(transport.connect().await?).await?;

    if let Some(config) = super::memory::policy().cloned() {
        super::memory_host::install(&connection, Arc::clone(&config)).await?;
    }
    super::tokenjuice_host::install(&connection).await?;

    Ok(ModuleRuntime {
        host,
        connection,
        handle: tokio::runtime::Handle::current(),
    })
}

/// Whether the module runtime has been stood up.
///
/// Lets status reporting answer without starting a broker as a side effect of
/// being asked a question.
#[must_use]
pub fn is_started() -> bool {
    RUNTIME.get().is_some()
}

#[cfg(test)]
#[path = "host_tests.rs"]
mod tests;
