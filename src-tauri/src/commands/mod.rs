pub mod auth;
pub mod chat;
pub mod memory;
pub mod model;
pub mod runtime;
pub mod socket;
pub mod openhuman;
pub mod unified_skills;

#[cfg(desktop)]
pub mod window;

// Re-export all commands for registration
pub use auth::*;
pub use chat::{chat_cancel, chat_send};
pub use memory::*;
pub use model::*;
pub use runtime::*;
pub use socket::*;
pub use openhuman::*;

#[cfg(desktop)]
pub use window::*;
