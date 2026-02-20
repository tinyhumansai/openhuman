pub mod auth;
pub mod model;
pub mod runtime;
pub mod socket;
pub mod tdlib;
pub mod alphahuman;

#[cfg(desktop)]
pub mod window;

// Re-export all commands for registration
pub use auth::*;
pub use model::*;
pub use runtime::*;
pub use socket::*;
pub use tdlib::*;
pub use alphahuman::*;

#[cfg(desktop)]
pub use window::*;
