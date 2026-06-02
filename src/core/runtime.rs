//! Shared tokio runtime tuning constants.
//!
//! A single agent turn is a very large async state machine (system prompt +
//! hundreds of tool specs + the nested provider/tool loop), and delegating
//! to a sub-agent runs another full turn one level down. Even with the inner
//! sub-agent future boxed, that nesting overflows tokio's default 2 MiB
//! worker-thread stack and aborts the whole process (SIGABRT:
//! "thread 'tokio-rt-worker' has overflowed its stack").
//!
//! PR #3155 set this on the standalone `openhuman-core run` JSON-RPC server.
//! Issue #3159 calls out that every other multi-thread runtime that can host
//! an agent turn (the desktop Tauri host's runtime, `agent_cli`, the rest of
//! `cli.rs`, …) shares the same exposure. Centralising the value keeps them
//! in sync; downstream call sites should set `.thread_stack_size(AGENT_WORKER_STACK_BYTES)`
//! on every multi-thread runtime that may host an agent turn.
pub const AGENT_WORKER_STACK_BYTES: usize = 16 * 1024 * 1024;
