//! Unified chat-provider factory.
//!
//! Resolves workload names (e.g. `"reasoning"`, `"heartbeat"`) to a
//! crate-native `ChatModel` plus the concrete model id selected for a workload.
//!
//! ## Provider-string grammar
//!
//! ```text
//! "openhuman"                    → OpenHumanBackendModel; model = config.default_model
//! "cloud" / missing              → primary_cloud; legacy custom inference_url wins when
//!                                  primary still points at OpenHuman after migration
//! "ollama:<model>[@<temp>]"      → local Ollama at config.local_ai.base_url
//! "lmstudio:<model>[@<temp>]"    → local LM Studio
//! "mlx:<model>[@<temp>]"         → local MLX-compatible server
//! "local-openai:<model>[@<temp>]"→ generic local OpenAI-compatible
//! "<slug>:<model>[@<temp>]"      → cloud_providers entry keyed by slug;
//!                                  builds the crate-native OpenAI client (Bearer) or
//!                                  Anthropic flavour depending on auth_style.
//! ```
//!
//! The optional `@<temp>` suffix pins a per-workload temperature override on
//! the built provider. The model id sent upstream never includes the suffix.
//!
//! Unknown slugs and missing-creds configurations produce actionable errors.

/// Test-only seam: inject a mock [`ChatModel`] so e2e tests can drive the
/// autonomous run paths (`spawn_workflow_run_background`, the task dispatcher)
/// with a scripted LLM and no network. Process-global because those runs are
/// detached `tokio::spawn`s — a thread/task-local would not reach them.
///
/// Because it is global, tests that install an override MUST run serially
/// and clear it via the returned guard. Inert in production: the check below
/// is gated on `cfg(test)` or an off-by-default test/profiling feature,
/// so the override is never consulted in shipped builds.
#[cfg(any(test, feature = "e2e-test-support", feature = "rss-bench"))]
#[path = "factory_test_provider_override_tests.rs"]
pub mod test_provider_override;

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "factory_tests.rs"]
mod factory_tests;
include!("factory_part_01.rs");
include!("factory_part_02.rs");
include!("factory_part_03.rs");
include!("factory_part_04.rs");
