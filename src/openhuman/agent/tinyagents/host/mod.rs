//! OpenHuman's implementations of the TinyAgents host capability traits.
//!
//! Each module here adapts one crate trait
//! ([`tinyagents::harness::host`]) onto the OpenHuman domains that actually
//! provide the behaviour. This is `docs/specs/plan-agents.md` **Phase 4**: the
//! agent runtime stops reaching into 45 domains directly and instead asks ten
//! capabilities, each of which is implemented here.
//!
//! # Why this is the valuable half
//!
//! Nothing has *moved* yet, and nothing needs to. Once `agent/` calls these
//! traits instead of the domains, its outbound coupling is ten seams rather
//! than forty-five — which is most of the architectural benefit of the
//! relocation with none of its risk. The plan explicitly allows the program to
//! stop here.
//!
//! # This is where policy lives
//!
//! The crate deliberately knows nothing about taint, scope, redaction,
//! approval, or egress budget. That is not an oversight — it is the boundary.
//! Every one of those guarantees is enforced in *these* files, on the way in
//! and out of the trait. An adapter that widens a permission or drops a scope
//! filter to make a signature fit would silently disable a guarantee the rest
//! of the system assumes, and no crate-side test could catch it.
//!
//! # Status
//!
//! Adapters exist and are tested; **`agent/` does not call them yet.**
//! Repointing the call sites is the remaining half of Phase 4 and is gated on
//! work tracked in the plan (the session still holds `AgentConfig` until
//! Phase 2's reader flip; `builder/factory.rs` is split rather than repointed).
//! Several methods carry `TODO(phase4)` where a domain surface was not
//! reachable — those are honest gaps, not stubs pretending to work, and each
//! names what is missing.

pub mod agent_memory;
pub mod budget_gate;
pub mod context_composer;
pub mod definition_registry;
pub mod experience_store;
pub mod learning_sink;
pub mod model_resolver;
pub mod progress_sink;
pub mod security_gate;
pub mod tool_outcome_classifier;

pub use agent_memory::OpenHumanAgentMemory;
pub use budget_gate::OpenHumanBudgetGate;
pub use context_composer::OpenHumanContextComposer;
pub use definition_registry::OpenHumanDefinitionRegistry;
pub use experience_store::OpenHumanExperienceStore;
pub use learning_sink::OpenHumanLearningSink;
pub use model_resolver::OpenHumanModelResolver;
pub use progress_sink::OpenHumanProgressSink;
pub use security_gate::OpenHumanSecurityGate;
pub use tool_outcome_classifier::OpenHumanToolOutcomeClassifier;
