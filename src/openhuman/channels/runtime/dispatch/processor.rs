//! Core message processing loop for the channel runtime.
//!
//! Contains:
//! * [`channel_has_approval_surface`] — gate controlling per-channel approval
//!   context scoping.
//! * [`try_route_approval_reply`] — intercepts yes/no approval replies before
//!   dispatching a fresh agent turn.
//! * [`process_channel_message`] — full per-message pipeline: typing, ACK
//!   reaction, history, agent turn, draft updates, reply.
//! * [`run_message_dispatch_loop`] — bounded-concurrency worker loop that feeds
//!   messages into [`process_channel_message`].

include!("processor_part_01.rs");
include!("processor_part_02.rs");
include!("processor_part_03.rs");
