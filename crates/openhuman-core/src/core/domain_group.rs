//! [`DomainGroup`]: the domain family each controller, tool, store and
//! subscriber belongs to.

/// Coarse-grained domain *family* a controller belongs to, used to gate its live
/// surface by the ambient [`crate::core::runtime::DomainSet`] (#4796).
///
/// Every registered controller is tagged with exactly one group at its single
/// registration site (`build_registered_controllers` /
/// `build_internal_only_controllers`); the live surface (schema dump,
/// dispatch, agent tools, stores, subscribers) filters by whether the active
/// [`crate::core::runtime::context::CoreContext`]'s `DomainSet` allows that
/// group. `full()` allows every group ⇒ registration is byte-identical to
/// pre-#4796. When no context is active (unit tests before boot) filtering is
/// disabled (treated as full).
///
/// The harness families (`Agent`/`Memory`/`Threads`/`Config`/`Security`) are on
/// under [`crate::core::runtime::DomainSet::harness`]; the gate families
/// (`Flows`/`Skills`/`Mcp`/`Channels`/`Web3`/`Voice`/`Media`) are the
/// per-feature axes the child issues (#4797–#4804) additionally narrow at
/// compile time. `Platform` is the catch-all for everything not in a named
/// family — always on in `full()`, off in `harness()`/`none()`.
///
/// **Groups track `crates/openhuman-core/src/` family directories 1:1.** Before the domain
/// reorg (#5328) they could not: a capability lived across up to 13 sibling
/// top-level dirs, so half the controller surface was tagged `Platform` for want
/// of a family to name. That made two things wrong which are now fixed:
///
/// - `harness()` claimed "agent + memory + threads + config + security" but
///   silently dropped `agent::{artifacts, learning}`,
///   `security::{credentials, devices}`, `config::workspace`,
///   `memory::people` and `skills::webhooks`, all of which sat in `Platform`.
/// - `embedded()` had to set `platform: true` purely to reach credentials and
///   config, which dragged in the desktop and hosted-backend surfaces it has no
///   use for. Those are now `Desktop` and `Hosted` and stay off.
///
/// `Platform` is now what its name says: the kernel surfaces with no family of
/// their own (`platform/`, `tools/`, `test_support/`).
///
/// When adding a family directory, add the matching variant here, a field on
/// [`crate::core::runtime::DomainSet`], an arm in `allows()`, and an entry in
/// each preset — the compiler enforces all four.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DomainGroup {
    // Harness families — on under `DomainSet::harness()`.
    Agent,
    Memory,
    Threads,
    Config,
    Security,
    // Gate families — off under `harness()`; per-gate Cargo features (#4797–#4804)
    // narrow these further at compile time.
    Flows,
    Skills,
    Mcp,
    Channels,
    Web3,
    Voice,
    Media,
    // Families carved out of the `Platform` catch-all once the domain reorg
    // (#5328) gave each one a directory to be named after. Before that, half the
    // controller surface was tagged `Platform` purely because there was no
    // family to point at — which made `DomainSet::embedded()` set
    // `platform: true` just to reach credentials/config/cron, dragging the
    // desktop and hosted-backend surfaces along with it.
    /// Model inference: providers, routing, local engines, embeddings, and the
    /// token-compression surface (`inference/`).
    Inference,
    /// External connectors reached on the user's behalf — Composio, calendar,
    /// file storage, task sources (`integrations/`).
    Integrations,
    /// Background initiative: scheduled cron jobs (`cron/`). Pairs with
    /// `ServiceSet::cron`.
    Automation,
    /// Code-execution substrate: the managed Node/Python runtimes, the worker
    /// pool, and the sandbox/CWD-jail confinement (`runtime/`, `sandbox/`).
    Runtimes,
    /// Desktop-shell-facing surfaces a headless or embedded host has no use for
    /// (`desktop/`).
    Desktop,
    /// Clients of the hosted TinyHumans backend — billing, team, referral, and
    /// announcements. Not built into the core: `openhuman-tinyhumans::hosted`
    /// registers them through [`register_controller_extension`](super::all::register_controller_extension), and this
    /// group is what the ambient `DomainSet` gates them with.
    Hosted,
    /// Loadable native modules: the module host, its registry, and the `modules`
    /// RPC surface (`modules/`).
    Modules,
    // Everything not in a named family — always on in `full()`, off otherwise.
    Platform,
    /// The SaaS operator plane: provisioning and inspecting user profiles
    /// (`profiles/`). On only under `DomainSet::saas()`.
    Operator,
}

impl DomainGroup {
    /// Number of variants. Kept in sync by `domain_group_all_lists_every_variant`.
    pub const COUNT: usize = 21;

    /// Every variant, for exhaustive iteration in drift guards.
    ///
    /// Hand-maintained, but not hand-*trusted*: [`DomainGroup::index`] below is
    /// an exhaustive `match`, so adding a variant is a compile error until it is
    /// given an index, and `domain_group_all_lists_every_variant` then fails
    /// until it appears here and [`COUNT`](Self::COUNT) is bumped. That chain is
    /// what makes the drift guards over `tool_group`, `StoreInitPlan` and
    /// `DomainSubscriberPlan` trustworthy — those three consume `DomainGroup`
    /// without the compiler checking coverage.
    pub const ALL: &'static [DomainGroup] = &[
        DomainGroup::Agent,
        DomainGroup::Memory,
        DomainGroup::Threads,
        DomainGroup::Config,
        DomainGroup::Security,
        DomainGroup::Flows,
        DomainGroup::Skills,
        DomainGroup::Mcp,
        DomainGroup::Channels,
        DomainGroup::Web3,
        DomainGroup::Voice,
        DomainGroup::Media,
        DomainGroup::Inference,
        DomainGroup::Integrations,
        DomainGroup::Automation,
        DomainGroup::Runtimes,
        DomainGroup::Desktop,
        DomainGroup::Hosted,
        DomainGroup::Modules,
        DomainGroup::Platform,
        DomainGroup::Operator,
    ];

    /// Dense index of this variant. Exhaustive by construction: the compiler
    /// rejects a newly added variant here, which is the first link in the chain
    /// described on [`ALL`](Self::ALL).
    pub const fn index(self) -> usize {
        match self {
            DomainGroup::Agent => 0,
            DomainGroup::Memory => 1,
            DomainGroup::Threads => 2,
            DomainGroup::Config => 3,
            DomainGroup::Security => 4,
            DomainGroup::Flows => 5,
            DomainGroup::Skills => 6,
            DomainGroup::Mcp => 7,
            DomainGroup::Channels => 8,
            DomainGroup::Web3 => 9,
            DomainGroup::Voice => 10,
            DomainGroup::Media => 11,
            DomainGroup::Inference => 12,
            DomainGroup::Integrations => 13,
            DomainGroup::Automation => 14,
            DomainGroup::Runtimes => 15,
            DomainGroup::Desktop => 16,
            DomainGroup::Hosted => 17,
            DomainGroup::Modules => 18,
            DomainGroup::Platform => 19,
            DomainGroup::Operator => 20,
        }
    }
}
