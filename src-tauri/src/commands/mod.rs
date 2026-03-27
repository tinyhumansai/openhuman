pub mod auth;
pub mod chat;
pub mod conscious_loop;
pub mod memory;
pub mod model;
pub mod openhuman;
pub mod runtime;
pub mod socket;
pub mod unified_skills;

#[cfg(desktop)]
pub mod window;

// Re-export all commands for registration
pub use auth::*;
pub use chat::{chat_cancel, chat_send};
pub use conscious_loop::conscious_loop_run;
pub use memory::*;
pub use model::*;
pub use openhuman::*;
pub use runtime::*;
pub use socket::*;

#[cfg(desktop)]
pub use window::*;
