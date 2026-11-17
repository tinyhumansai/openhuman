//! [`DomainSet`]: which domain families a core serves.

use crate::core::all::DomainGroup;

/// Selects which domain *families* exist at runtime on a [`CoreRuntime`](super::CoreRuntime) (#4796).
///
/// Sibling of [`ServiceSet`]: where `ServiceSet` selects background services and
/// transports, `DomainSet` selects which controller/tool/store/subscriber
/// surfaces are live. Each flag is an independent [`DomainGroup`]; presets cover
/// the common hosts:
/// [`DomainSet::full`] (every family — today's behavior, the default),
/// [`DomainSet::harness`] (agent + memory + threads + config + security only —
/// the embeddable agent core used by `crates/openhuman-embed/examples/lean_headless.rs`), and
/// [`DomainSet::none`] (all domain families disabled; transport built-ins and
/// always-on core infrastructure still run).
///
/// `full()` is byte-identical to pre-#4796 registration, so the desktop shell
/// and standalone CLI are unchanged. Per-gate Cargo `[features]` (children
/// #4797–#4804) narrow the *compile-time* surface further; this struct is the
/// *runtime* axis they compose with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DomainSet {
    /// Agent definition/registry/experience, orchestration, session DB/import.
    pub agent: bool,
    /// Documents, knowledge graph, memory tree/sources/sync/diff/goals.
    pub memory: bool,
    /// Conversation threads, per-thread goals, todos.
    pub threads: bool,
    /// Persisted runtime configuration.
    pub config: bool,
    /// Encryption, keyring consent, security policy, approval, plan-review.
    pub security: bool,
    /// Saved automation workflows (tinyflows graphs).
    pub flows: bool,
    /// SKILL.md skills, skill runtime, skill registry.
    pub skills: bool,
    /// MCP client subsystem (Smithery registry, local servers, audit).
    pub mcp: bool,
    /// Messaging channels + webview bridges (web channel, whatsapp data, …).
    pub channels: bool,
    /// Wallet, high-level web3 surface, x402 machine payments.
    pub web3: bool,
    /// Speech-to-text / text-to-speech, audio toolkit.
    pub voice: bool,
    /// Image/video media generation. NOTE: today this gates only the
    /// `media_generate_*` **agent tools** — no controller/store/subscriber is
    /// tagged `Media` (there is no `media` RPC namespace yet), so a custom set
    /// with `media: false, platform: true` drops the media tools while any
    /// future backing controller would stay live. Fold the media-generation
    /// controller into this group when it lands.
    pub media: bool,
    /// Model inference: providers, routing, local engines, embeddings.
    pub inference: bool,
    /// External connectors (Composio, calendar, file storage, task sources).
    pub integrations: bool,
    /// Background initiative: scheduled cron jobs.
    pub automation: bool,
    /// Code-execution substrate: Node/Python runtimes, pool, sandbox.
    pub runtimes: bool,
    /// Desktop-shell-facing surfaces.
    pub desktop: bool,
    /// Clients of the hosted TinyHumans backend.
    pub hosted: bool,
    /// Loadable native modules: the module host, registry and `modules` RPC.
    pub modules: bool,
    /// Everything not in a named family — always on in `full()`.
    pub platform: bool,
    /// The SaaS operator plane (`profiles.*`). Off in every preset but
    /// `DomainSet::saas()`, `full()` included: a single-user core has no
    /// users to provision.
    pub operator: bool,
}

impl DomainSet {
    /// Every family on — today's behavior and the [`CoreBuilder`](super::CoreBuilder) default.
    /// Registration is byte-identical to pre-#4796.
    pub fn full() -> Self {
        Self {
            agent: true,
            memory: true,
            threads: true,
            config: true,
            security: true,
            flows: true,
            skills: true,
            mcp: true,
            channels: true,
            web3: true,
            voice: true,
            media: true,
            inference: true,
            integrations: true,
            automation: true,
            runtimes: true,
            desktop: true,
            hosted: true,
            modules: true,
            platform: true,
            operator: false,
        }
    }

    /// The embeddable agent core: agent + memory + threads + config + security.
    /// Every gate family AND `platform` are off. Used by
    /// `crates/openhuman-embed/examples/lean_headless.rs`.
    pub fn harness() -> Self {
        Self {
            agent: true,
            memory: true,
            threads: true,
            config: true,
            security: true,
            flows: false,
            skills: false,
            mcp: false,
            channels: false,
            web3: false,
            voice: false,
            media: false,
            inference: false,
            integrations: false,
            automation: false,
            runtimes: false,
            desktop: false,
            hosted: false,
            modules: false,
            platform: false,
            operator: false,
        }
    }

    /// A long-lived embedded host: the harness core plus the workflow engine
    /// and the supporting
    /// runtime, automation, integration, and platform surfaces it needs.
    ///
    /// Named for the *shape* rather than any downstream consumer — the core
    /// does not know which host embeds it, and a preset naming one would invert
    /// that. Suits any process that drives the core in-process through the
    /// typed facade and owns its own presentation layer.
    ///
    /// Deliberately NOT built on [`DomainSet::harness`]: that preset sets
    /// `platform: false`, which drops credentials, config, cron, task_sources
    /// and todos, and leaves `channels` off — but `channel.web_chat` is tagged
    /// `DomainGroup::Channels` and an embedded host drives chat turns through it.
    ///
    /// `flows: true` keeps the tinyflows engine and boot reconciliation on;
    /// reconciliation keys off
    /// `ctx.domains().flows` rather than a `ServiceSet` flag.
    ///
    /// An embedded host supplies its own harness wrappers, networking and
    /// routing, so `web3` / `voice` / `media` / `mcp` stay off.
    pub fn embedded() -> Self {
        Self {
            agent: true,
            memory: true,
            threads: true,
            config: true,
            security: true,
            flows: true,
            skills: true,
            mcp: false,
            channels: true,
            web3: false,
            voice: false,
            media: false,
            inference: true,
            integrations: true,
            automation: true,
            runtimes: true,
            desktop: false,
            hosted: false,
            modules: false,
            platform: true,
            operator: false,
        }
    }

    /// The kernel floor: threads, config, security — and nothing else.
    ///
    /// Distinct from [`DomainSet::none`], which is "no domains at all". This is
    /// "the minimum a host needs before opting a subsystem back in", so an
    /// embedder can request kernel + exactly one family. `agent` and `memory`
    /// are OFF on purpose: they are the two largest subsystems and the ones an
    /// alternative driver would replace, so a host that wants them says so.
    ///
    /// See `crates/openhuman-embed/examples/capability_report.rs`.
    pub fn kernel() -> Self {
        Self {
            agent: false,
            memory: false,
            threads: true,
            config: true,
            security: true,
            flows: false,
            skills: false,
            mcp: false,
            channels: false,
            web3: false,
            voice: false,
            media: false,
            inference: false,
            integrations: false,
            automation: false,
            runtimes: false,
            desktop: false,
            hosted: false,
            modules: false,
            platform: false,
            operator: false,
        }
    }

    /// Nothing on — every family disabled.
    pub fn none() -> Self {
        Self {
            agent: false,
            memory: false,
            threads: false,
            config: false,
            security: false,
            flows: false,
            skills: false,
            mcp: false,
            channels: false,
            web3: false,
            voice: false,
            media: false,
            inference: false,
            integrations: false,
            automation: false,
            runtimes: false,
            desktop: false,
            hosted: false,
            modules: false,
            platform: false,
            operator: false,
        }
    }

    /// Whether the given [`DomainGroup`] is enabled in this set.
    pub fn allows(&self, group: DomainGroup) -> bool {
        match group {
            DomainGroup::Agent => self.agent,
            DomainGroup::Memory => self.memory,
            DomainGroup::Threads => self.threads,
            DomainGroup::Config => self.config,
            DomainGroup::Security => self.security,
            DomainGroup::Flows => self.flows,
            DomainGroup::Skills => self.skills,
            DomainGroup::Mcp => self.mcp,
            DomainGroup::Channels => self.channels,
            DomainGroup::Web3 => self.web3,
            DomainGroup::Voice => self.voice,
            DomainGroup::Media => self.media,
            DomainGroup::Inference => self.inference,
            DomainGroup::Integrations => self.integrations,
            DomainGroup::Automation => self.automation,
            DomainGroup::Runtimes => self.runtimes,
            DomainGroup::Desktop => self.desktop,
            DomainGroup::Hosted => self.hosted,
            DomainGroup::Modules => self.modules,
            DomainGroup::Platform => self.platform,
            DomainGroup::Operator => self.operator,
        }
    }

    /// Field-wise AND with `other`: a family is on in the result only if it
    /// was on in both.
    ///
    /// Used to clamp a derived context's requested domains to what the
    /// parent context actually registered — see
    /// [`CoreContext::derive_with`](crate::core::runtime::CoreContext::derive_with).
    /// A derived overlay is meant to *narrow* the parent, never state a
    /// family the parent never registered back into existence.
    #[must_use]
    pub fn intersect(&self, other: &DomainSet) -> DomainSet {
        DomainSet {
            agent: self.agent && other.agent,
            memory: self.memory && other.memory,
            threads: self.threads && other.threads,
            config: self.config && other.config,
            security: self.security && other.security,
            flows: self.flows && other.flows,
            skills: self.skills && other.skills,
            mcp: self.mcp && other.mcp,
            channels: self.channels && other.channels,
            web3: self.web3 && other.web3,
            voice: self.voice && other.voice,
            media: self.media && other.media,
            inference: self.inference && other.inference,
            integrations: self.integrations && other.integrations,
            automation: self.automation && other.automation,
            runtimes: self.runtimes && other.runtimes,
            desktop: self.desktop && other.desktop,
            hosted: self.hosted && other.hosted,
            modules: self.modules && other.modules,
            platform: self.platform && other.platform,
            operator: self.operator && other.operator,
        }
    }
}
