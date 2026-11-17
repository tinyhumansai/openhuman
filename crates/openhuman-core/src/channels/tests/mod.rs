//! Cross-channel integration suite, declared as
//! `#[cfg(all(feature = "channels", test))] mod tests` in `channels/mod.rs`
//! (see AGENTS.md — unit tests otherwise stay beside their modules as
//! `*_tests.rs`; this tree is the deliberate exception for tests that
//! exercise more than one channel submodule end-to-end).
//!
//! `common.rs` holds the shared fixtures the files below build on: fake
//! models per scenario (`DummyModel`, `SlowModel`, `ToolCallingModel`,
//! `IterativeToolModel`, `HistoryCaptureModel`, `ModelCaptureModel`), a
//! concrete test channel handle that records sends and typing calls,
//! `MockPriceTool`, `make_workspace` for identity-file
//! fixtures, and a re-export of `agent::bus::use_real_agent_handler`.
//!
//! * `identity.rs` — `build_system_prompt` inlining workspace identity
//!   markdown into the Project Context section.
//! * `prompt.rs` — system-prompt section assembly and bootstrap truncation.
//! * `host_pipeline.rs` — thread/reaction routing through the host send path
//!   and thread-scoped conversation histories across Discord channels.
//! * `runtime_dispatch.rs` — the dispatch loop through
//!   `runtime::test_support::run_dispatch_harness`: inbound-envelope
//!   publication, parallel processing, typing-task cancellation, the
//!   `agent.run_turn` bus route, and the dispatch deadline.
//! * `runtime_hardening.rs` — multimodal file and relative path-marker
//!   hardening through `process_channel_message`.
//! * `runtime_tool_calls.rs` — native tool-call execution, the `/models`
//!   command short-circuit, route overrides, and `max_tool_iterations`
//!   through `process_channel_message`.

mod common;
mod host_pipeline;
mod identity;
mod prompt;
mod runtime_debounce;
mod runtime_dispatch;
mod runtime_hardening;
mod runtime_tool_calls;
