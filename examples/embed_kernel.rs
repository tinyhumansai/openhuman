//! Embed the OpenHuman core at its **kernel floor**, then opt one subsystem in.
//!
//! [`DomainSet::kernel`] is the smallest useful runtime surface: threads,
//! config, and security — the transport, dispatch, policy and identity a host
//! needs before it has decided what the core is *for*. Notably `agent` and
//! `memory` are OFF: they are the two largest subsystems and the ones an
//! alternative driver would replace, so a host that wants them says so.
//!
//! Contrast the two presets:
//!
//! - [`DomainSet::harness`] — kernel + `agent` + `memory`. The embeddable agent
//!   core; see `examples/embed_headless.rs`.
//! - [`DomainSet::kernel`] — the floor. Start here when you want, say, memory
//!   without the agent harness, or when you intend to bind your own driver to a
//!   subsystem slot.
//!
//! Because `DomainSet` is a plain struct, opting a family back in is a field
//! assignment — no builder ceremony:
//!
//! ```ignore
//! let mut domains = DomainSet::kernel();
//! domains.memory = true;          // kernel + memory, nothing else
//! ```
//!
//! What "off" means is uniform across every axis: the family's controllers are
//! unregistered (unknown-method over `/rpc`, absent from `/schema`), its agent
//! tools are absent from the tool list rather than present-and-failing, and its
//! stores and event-bus subscribers never initialize.
//!
//! Run with:
//!
//! ```bash
//! GGML_NATIVE=OFF cargo run --example embed_kernel
//! ```

use openhuman_core::{CoreBuilder, DomainSet, HostKind, ServiceSet};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Kernel floor plus exactly one subsystem: memory. The agent harness stays
    // out, so this process can serve memory reads/writes without ever
    // constructing an agent.
    let mut domains = DomainSet::kernel();
    domains.memory = true;

    let runtime = CoreBuilder::new(HostKind::Cli)
        .domains(domains)
        .services(ServiceSet::none())
        .build()
        .await?;

    // Always available — `core.*` is kernel transport, not a domain.
    let version = runtime
        .invoke("core.version", serde_json::json!({}))
        .await
        .map_err(|e| anyhow::anyhow!("core.version failed: {e}"))?;
    println!("core.version -> {version}");

    // Enabled: memory was opted in above.
    match runtime
        .invoke("openhuman.memory_list_namespaces", serde_json::json!({}))
        .await
    {
        Ok(v) => println!("memory_list_namespaces -> {v}"),
        Err(e) => println!("memory_list_namespaces -> error: {e:?}"),
    }

    // Disabled: `agent` is off under kernel(), so this is an UNKNOWN METHOD —
    // not a registered handler returning "agent disabled". Absence is the
    // contract: a registered-but-failing method teaches a model the capability
    // exists and makes it retry.
    match runtime
        .invoke("openhuman.agent_list_definitions", serde_json::json!({}))
        .await
    {
        Ok(v) => println!("agent_list_definitions -> unexpectedly OK: {v}"),
        Err(e) => println!("agent_list_definitions -> unknown method (expected): {e:?}"),
    }

    Ok(())
}
