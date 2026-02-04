pub mod auth;
pub mod runtime;
pub mod socket;
pub mod tdlib;

#[cfg(desktop)]
pub mod window;

// Re-export all commands for registration
pub use auth::*;
pub use runtime::*;
pub use socket::*;
pub use tdlib::*;

#[cfg(desktop)]
pub use window::*;
