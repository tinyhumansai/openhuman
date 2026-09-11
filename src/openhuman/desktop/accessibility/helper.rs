//! Unified Swift helper process: focus queries, paste, and overlay in one native binary.
//!
//! Replaces the separate osascript subprocess spawns and standalone overlay binary
//! with a single persistent Swift process communicating via stdin/stdout JSON.
//!
//! ## Mutex architecture
//!
//! Three globals prevent deadlock between fire-and-forget (show/hide) and
//! request-response (focus/paste) callers:
//!
//! - `UNIFIED_HELPER`: guards the process handle + stdin writer.
//!   Held only for the brief duration of a stdin write (~μs).
//! - `RESPONSE_RX`: guards the mpsc receiver that the background reader
//!   thread populates.  Held only for the duration of `recv_timeout`.
//! - `RECV_SERIALISER`: held for the entire send+receive round-trip so that
//!   two callers cannot interleave their reads.
//!
//! Fire-and-forget callers never touch `RESPONSE_RX` or `RECV_SERIALISER`,
//! so `show`/`hide` can proceed while a `focus` query is in-flight.

include!("helper_part_01.rs");
include!("helper_part_02.rs");
