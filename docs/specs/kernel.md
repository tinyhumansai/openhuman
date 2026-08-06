# OpenHuman as a Kernel — Subsystem & Driver Model

**Status:** proposed · **Date:** 2026-07-28 · **Scope:** `src/` (core crate), all `src/openhuman/*` domains
**Companion spec:** [`plan-memory.md`](plan-memory.md) — memory is the first subsystem cut to this model.

---

## 1. Thesis

Linux is a good kernel because it does not implement filesystems, network cards, or
schedulersuites — it defines **narrow contracts** (VFS, netdev, block layer), owns **policy and
mechanism** (permissions, namespaces, scheduling, lifecycle), and lets independently-developed
**drivers** implement the actual behaviour behind those contracts. A driver can be built in, a
module, or absent; userspace never learns which.

`openhuman-core` should be that kernel for a personal AI runtime. Today it is closer to a
monolith with one very good in-tree implementation per capability: memory *is* TinyCortex, agents
*are* TinyAgents, channels *are* TinyChannels. Each already has a seam (`src/openhuman/tinycortex/`,
`src/openhuman/tinyagents/`), which proves the shape works — but the seams are **bespoke per
domain**, the contracts are **not versioned**, and there is **no way for a third implementation to
be bound at runtime**.

This spec defines the general model. It deliberately generalises patterns the repo already has
rather than inventing new ones.

**Non-goal:** a plugin marketplace, dynamic `.so` loading, or an ABI. Drivers are Rust crates
compiled in, or out-of-process services reached over a documented wire contract. Nothing here
requires unsafe dynamic linking.

---

## 2. What already exists (the raw material)

| Kernel concern | Existing mechanism | Gap for the kernel model |
| --- | --- | --- |
| Syscall surface | Controller registry (`src/core/all.rs`), JSON-RPC `/rpc`, `/schema` | Method set is fixed at compile time; cannot vary by bound driver |
| Runtime composition | `DomainSet` / `DomainGroup` on `CoreBuilder` | Selects *whether* a domain runs, not *which implementation* |
| Build composition | Per-domain Cargo `[features]` (`voice`, `web3`, `mcp`, `channels`, …) | Gate = on/off, not a choice between implementations |
| Service lifecycle | `ServiceSet`, `src/core/runtime/services.rs` | No per-subsystem health/degraded state |
| IPC | `event_bus/` broadcast + native request/response | Fine as-is; becomes the kernel's internal bus |
| Policy | `SecurityPolicy`, approval gate, `MemoryTaint`, `source_scope`, redaction | Enforced *inside* domains, so a swapped implementation could bypass it |
| Trust metadata | `CapabilityProviderConfig` (`config/schema/capability_providers.rs`) | Already the right shape; unused by domains |
| Seams | `src/openhuman/tinycortex/`, `src/openhuman/tinyagents/` | Adapter to *one* crate, not to a trait a second crate could also satisfy |

The kernel model is mostly **naming and enforcing** the above, plus one genuinely new piece: the
**subsystem registry with a bound driver per slot**.

---

## 3. Model

### 3.1 Definitions

- **Kernel** — `src/core/` plus the always-on platform domains. Owns: RPC transport and the
  controller registry, the event bus, config load/validation, `SecurityPolicy` and the approval
  gate, scheduling/cron, the workspace and path roots, observability, and the subsystem registry.
  The kernel contains **no capability implementation**.
- **Subsystem** — a named capability slot: `memory`, `inference`, `channels`, `skills`, `flows`,
  `sandbox`, `voice`. Each subsystem owns a **contract** (a set of Rust traits + value types), a
  **config section**, a **stable RPC namespace**, and an **agent-tool family**.
- **Driver** — an implementation of a subsystem contract. Three classes:
  - **`embedded`** — an in-tree/vendored Rust crate (`tinycortex`, `tinyagents`, `tinychannels`).
    The default; no network, no extra process.
  - **`external`** — an out-of-process backend reached through a transport adapter over a
    documented wire contract (HTTP/JSON, or MCP). This is how a third party ships a driver
    without touching this repo.
  - **`null`** — a stub advertising zero capabilities. What a compiled-out or unconfigured
    subsystem binds to. Replaces today's hand-written `stub.rs` files with one generic answer.
- **Binding** — exactly **one** driver is bound per subsystem per process, chosen by config at
  boot. Same rule OpenClaw uses for `plugins.slots.memory`: installing a second memory backend
  disables the first with a warning, because two live memory backends means two truths. Fan-out
  across several backends is expressed as a **composite driver** (§3.5), not as a second binding.

### 3.2 The contract shape (normative)

Every subsystem contract is defined in an **API module with no engine dependencies** and follows
the same five-part shape:

```rust
// 1. Identity + lifecycle — every driver implements this.
#[async_trait]
pub trait Driver: Send + Sync + 'static {
    fn id(&self) -> &str;                       // "tinycortex", "supermemory", "null"
    fn class(&self) -> DriverClass;             // Embedded | External | Null
    fn capabilities(&self) -> Capabilities;     // 2.
    async fn health(&self) -> DriverHealth;     // Ready | Degraded { reason } | Down { reason }
    async fn shutdown(&self) -> Result<()>;
}

// 2. Capability descriptor — a bitset/struct of optional trait families, not a version number.
// 3. Capability traits — one per family; a driver implements only what it advertises.
// 4. Value types — serde-only, dependency-free, shared by every driver.
// 5. Errors — a typed enum with a mandatory `Unsupported { capability }` variant.
```

**Rules:**

1. **Capabilities are negotiated, not assumed.** The kernel asks `capabilities()` once at bind
   time and caches it. Calling an unadvertised capability is a kernel bug, not a driver error.
2. **Value types are inert.** Serde/std only — no SQLite, no tokio-specific types, no engine
   types. This is the same carve-out rule the `skills` and `mcp` Cargo gates already follow
   (`AGENTS.md`: *"put a domain's inert types in a dep-free submodule and leave it ungated"*), now
   applied across crate boundaries so an external driver can depend on the API without pulling
   the embedded engine.
3. **Drivers never see kernel concerns.** No RPC schemas, no `SecurityPolicy`, no keychain, no
   event bus, no `Config`. Everything a driver needs is passed in its constructor or per call.
4. **Contracts are versioned.** Each API module carries `pub const CONTRACT_VERSION: (u16, u16)`.
   Minor bump = capability added; major bump = existing signature changed. External drivers
   report the version they speak in their handshake; a major mismatch fails the bind.

### 3.3 Degradation is a first-class outcome

When a bound driver does not advertise a capability, the kernel does **not** register a handler
that returns "not implemented". It behaves exactly like today's compile-time gates:

- the corresponding **RPC methods are unregistered** — unknown-method over `/rpc`, absent from
  `/schema`;
- the corresponding **agent tools are absent** from the tool list, not present-and-failing;
- the **UI** reads the capability set from `<subsystem>_status` and hides the surface.

Absence beats a stub that errors. A registered-but-failing method teaches the model that the
capability exists and makes it retry (the exact reasoning already recorded for the `flows` gate).
The one exception is the **CLI**, which keeps its subcommand arm and reports a *build/config fact*
("memory driver `supermemory` does not support tree summarisation") — same reasoning as the
retained `mcp` and `tui` CLI arms.

### 3.4 Policy is kernel-side and non-bypassable

Every subsystem call from product code goes through a kernel-owned **guard decorator**, never to
the driver directly:

```text
agent tool / RPC handler
        │
        ▼
  Guard<D>  ── SecurityPolicy · taint stamping · scope allowlist · redaction ·
        │      egress budget · approval gate · audit event · tracing span
        ▼
   bound driver D
```

`Guard` implements the same contract traits as `D`, so it is transparent to callers and
impossible to skip by construction. This closes the single largest risk of a driver model: today
`MemoryTaint`, `source_scope`, and redaction are enforced *inside* the memory domain, so a
replacement implementation would silently drop them. After this change a driver **cannot** see
un-redacted content it was not granted, and cannot stamp its own provenance.

**External drivers additionally require:** a `CapabilityProviderConfig` entry with an explicit
`trust_state` (fail-closed `untrusted`), a recorded egress decision, and per-call budget
accounting. Sending user memory to a hosted backend is an egress event and is treated as one.

### 3.5 Composition instead of kernel special-cases

Multi-backend behaviour is expressed as drivers that wrap drivers:

- **`composite`** — fan out reads across N drivers, merge/rank, write to a designated primary.
- **`mirror`** — write to both, read from primary; the migration path between backends.
- **`cache`** — embedded driver in front of an external one.

Each is just another `Driver`, so the kernel keeps exactly one bind and zero special cases.

### 3.6 Config shape (uniform across subsystems)

```toml
[subsystems.memory]
driver = "tinycortex"           # the bound slot; "null" disables

[subsystems.memory.drivers.tinycortex]
# embedded driver options

[subsystems.memory.drivers.supermemory]
class     = "external"
transport = "http"
endpoint  = "https://…"
credential_ref = "keychain:supermemory"   # never an inline secret
```

Secrets are **references**, resolved kernel-side through the existing keychain, and passed to the
driver as a redacted `SecretString` — the pattern already pinned for Composio credentials.

### 3.7 Runtime axes (unchanged, now three)

| Axis | Question | Mechanism |
| --- | --- | --- |
| Compile-time | Is this code in the binary? | Cargo `[features]` |
| Runtime composition | Does this domain run this process? | `DomainSet` / `ServiceSet` |
| **Binding (new)** | **Which implementation answers?** | **subsystem registry + config** |

They compose: a subsystem compiled out binds `null`; a subsystem gated off by `DomainSet` is not
bound at all; a subsystem present and enabled binds the configured driver, falling back to the
embedded default if that driver fails to construct (logged loudly, surfaced in status, never
silent).

> **Feature-forwarding gate applies.** Any new default-ON gate (e.g. `memory-embedded`) must be
> added to `app/src-tauri/Cargo.toml`'s explicit feature list — the shell sets
> `default-features = false`. `scripts/ci/check-feature-forwarding.mjs` enforces this; the `voice`
> and `tokenjuice-treesitter` incidents are why.

---

## 4. Kernel/driver split criterion

One question decides where any file lives:

> **Would a build whose only driver is a third-party external backend still need this file?**

- **Yes → kernel.** RPC schemas and ops, agent-tool definitions, `SecurityPolicy` and policy
  guards, provenance/taint, scope and redaction, credentials and keychain, schedulers and cron,
  the event bus, config mapping, the driver registry and transport adapters, export/import.
- **No → driver crate.** Storage engines, indexes, chunking, embeddings pipelines, retrieval and
  ranking, summary trees, job engines, source readers and parsers, provider-specific
  normalisation, on-disk formats and migrations.

This is a sharper rule than "is it product policy or engine logic", which is how the
2026-07-28 cutover evaluation landed on keeping several engine-shaped modules in the host. Under
the kernel criterion those modules are **implementation of the default driver** and belong to it —
see the memory spec §6 for the concrete re-disposition.

---

## 5. Subsystem roadmap

| Subsystem | Default driver | Contract status | Order |
| --- | --- | --- | --- |
| **memory** | `tinycortex` (embedded) | To be defined — companion spec | **1st (pilot)** |
| inference | `tinyagents` routing | Partly exists (`routing`, provider traits) | 2nd |
| channels | `tinychannels` | Trait exists (`channels::traits`, already an ungated carve-out) | 3rd |
| sandbox | local OS jail | Already trait-shaped (Docker / Landlock / Noop) | 4th — smallest, good validation |
| skills · flows | in-tree | Gated already; contract later | later |

Memory goes first: it has the most mature seam, a golden-workspace parity harness, and a real
external demand (pluggable backends such as Supermemory/mem0).

---

## 6. Definition of done (kernel layer)

1. `src/core/subsystem/` exists: `Driver`, `DriverClass`, `DriverHealth`, `Capabilities`,
   `SubsystemRegistry`, `Guard`, and the `[subsystems.*]` config mapping.
2. Binding happens once at `CoreBuilder` time; a failed bind falls back to the embedded default,
   emits a `DomainEvent`, and is visible in `<subsystem>_status`.
3. Controller registration in `src/core/all.rs` is filtered by the bound driver's capability set,
   the same way it is filtered by `DomainSet` today.
4. Agent-tool assembly is filtered by the same set.
5. `Guard` is the only path from product code to a driver; a test asserts no direct driver call
   site exists outside the registry module.
6. `openhuman subsystems` CLI + `subsystems_status` RPC list slot, bound driver, class, health,
   contract version, and capabilities.
7. Docs: `gitbooks/developing/architecture/kernel.md` describes the model; `AGENTS.md` gains a
   "adding a subsystem driver" checklist.

## 7. Risks

- **Capability sprawl.** Every optional trait is a branch in RPC registration and tool assembly.
  Mitigation: capability families are coarse (≤10 per subsystem) and adding one requires a
  contract minor bump plus a both-ways test.
- **Guard bypass.** Mitigation: drivers are private to the registry module; a lint test greps for
  out-of-module construction.
- **Parity regressions.** Mitigation: the golden-workspace harness that already backs the
  TinyCortex cutover is promoted to the general conformance suite (§ memory spec 7).
- **Over-abstraction.** Mitigation: memory ships end-to-end and a second real driver exists before
  a second subsystem is cut over. One proven seam beats five speculative ones.

