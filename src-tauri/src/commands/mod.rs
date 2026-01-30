pub mod auth;
pub mod socket;
pub mod telegram;
pub mod window;

// Re-export all commands for registration
pub use auth::*;
pub use socket::*;
pub use telegram::*;
pub use window::*;
