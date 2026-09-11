//! WebSocket Engine.IO / Socket.IO connection loop with automatic reconnection.

#[cfg(test)]
#[path = "ws_loop_tests.rs"]
mod tests;
include!("ws_loop_part_01.rs");
include!("ws_loop_part_02.rs");
