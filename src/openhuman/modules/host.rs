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
//! # The runtime that gets here first owns the bus
//!
//! The broker and the connection are tokio tasks, so they belong to whichever
//! runtime calls [`runtime`] first. In the core that is the one runtime the
//! process has, and the question never arises.
//!
//! It arises in tests. Two `#[tokio::test]` functions each build their own
//! runtime, and the second one to call a loaded module finds a broker whose tasks
//! died with the first runtime — the call does not fail, it hangs until whatever
//! deadline is above it fires. Any test that drives a real module therefore has
//! to be the only one in its process, which is why the module-backed tool tests
//! are `#[ignore]`d rather than merely gated on an artifact being present.

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

    let runtime = ModuleRuntime { host, connection };
    // A concurrent caller may have won the race. Its runtime is equivalent, so
    // take the winner's and let ours drop.
    Ok(RUNTIME.get_or_init(|| runtime))
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
mod tests {
    use super::{is_started, runtime};

    /// Everything that touches the process-global module runtime, in one test.
    ///
    /// One test and not several, on purpose: `runtime()` is a process-global
    /// started by whichever tokio runtime reaches it first, and each
    /// `#[tokio::test]` builds its own. A second test function would find a
    /// broker whose tasks died with the runtime that spawned it, and its call
    /// would hang until something above it timed out rather than failing — the
    /// same affinity hazard the module spec documents for the module-backed tool
    /// tests. Splitting these up would reintroduce it.
    #[tokio::test]
    async fn the_module_bus_is_a_singleton_and_serves_proxies() {
        let first = runtime().await.expect("runtime should start");
        assert!(is_started());
        let second = runtime().await.expect("runtime should be reused");
        assert!(
            std::ptr::eq(first, second),
            "runtime() handed out two different runtimes"
        );

        // Building a proxy is a local operation — it validates names and nothing
        // else. Nothing has claimed the name, so the call fails rather than
        // hanging, which is what makes `ensure_loaded` worth having.
        let proxy = first
            .proxy(
                "ai.tinyhumans.tinydocs.Documents",
                "/ai/tinyhumans/tinydocs/Documents",
            )
            .expect("registry names should be well formed");
        let result: tinybus::Result<serde_json::Value> = proxy.call("GenerateDocx", ()).await;
        assert!(result.is_err(), "an unloaded module should not answer");

        // A name that cannot be a bus name is refused without reaching the bus.
        assert!(first.proxy("not a bus name", "/nope").is_err());
    }
}
