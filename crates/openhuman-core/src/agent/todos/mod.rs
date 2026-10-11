//! OpenHuman adapters around the TinyAgents todo store.
//!
//! Design notes:
//! - **Per-session scoped.** The list belongs to the agent session the turn
//!   runs in; there is no cross-session or app-wide list.
//! - **In-memory scratch.** When no session context is available the
//!   process-global scratch store is used (tool invocations outside a chat
//!   session, tests).
//! - **Markdown output.** Tool results include a rendered representation for
//!   the agent transcript.

pub mod ops;
