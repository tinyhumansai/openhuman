//! Cross-channel integration suite, declared as
//! `#[cfg(all(feature = "channels", test))] mod tests` in `channels/mod.rs`
//! (see AGENTS.md — unit tests otherwise stay beside their modules as
//! `*_tests.rs`; this tree is the deliberate exception for tests that
//! exercise more than one channel submodule end-to-end).
//!
//! `common.rs` holds the shared fixtures the files below build on: fake
//! models per scenario (`DummyModel`, `SlowModel`, `ToolCallingModel`,
//! `IterativeToolModel`, `HistoryCaptureModel`, `ModelCaptureModel`), fake
//! `Channel` implementations that record sends or always fail
//! (`RecordingChannel`, `TelegramRecordingChannel`, `AlwaysFailChannel`), a
//! `MockPriceTool`, `make_workspace` for identity-file
//! fixtures, and a re-export of `agent::bus::use_real_agent_handler`.
//!
//! * `discord_integration.rs` — end-to-end dispatch through the Discord
//!   channel with every cross-module boundary (agent runtime, provider) substituted, proving the domain stays encapsulated.
//! * `health.rs` — `spawn_supervised_listener` marking a component errored and restarting
//!   when a listener keeps failing.
//! * `identity.rs` — `build_system_prompt` inlining workspace identity
//!   markdown into the Project Context section.
//! * `prompt.rs` — system-prompt section assembly and bootstrap truncation.
//! * `runtime_dispatch.rs` — the dispatch loop through
//!   `runtime::test_support::run_dispatch_harness`: inbound-envelope
//!   publication, parallel processing, typing-task cancellation, the
//!   `agent.run_turn` bus route, and multimodal path-marker hardening.
//! * `runtime_tool_calls.rs` — native tool-call execution, the `/models`
//!   command short-circuit, route overrides, and `max_tool_iterations`
//!   through `process_channel_message`.
//! * `telegram_integration.rs` — Telegram reactions, reply/thread roundtrip,
//!   and typing-indicator lifecycle against a recording channel.

mod common;
mod discord_integration;
mod health;
mod identity;
mod prompt;
mod runtime_dispatch;
mod runtime_tool_calls;
mod telegram_integration;
