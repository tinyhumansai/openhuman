//! The signed-in user as the core knows it, reduced to the fields a host may
//! attach to diagnostics (`id`, `name`, `email`).

pub use openhuman_core::agent::prompts::UserIdentity;
pub use openhuman_core::credentials::identity::peek_credential_user_identity;
