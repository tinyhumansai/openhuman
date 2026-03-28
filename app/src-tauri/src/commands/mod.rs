pub mod chat;
pub mod conscious_loop;
pub mod core_relay;
pub mod model;
pub mod openhuman;
pub mod runtime;

#[cfg(desktop)]
pub mod window;

// Re-export all commands for registration
pub use chat::{chat_cancel, chat_send};
pub use conscious_loop::conscious_loop_run;
pub use core_relay::*;
pub use model::*;
pub use openhuman::*;
pub use runtime::*;

#[cfg(desktop)]
pub use window::*;
