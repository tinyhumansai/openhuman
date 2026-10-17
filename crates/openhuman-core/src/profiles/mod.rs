//! Profiles: in SaaS mode, each user is served as one profile.
//!
//! A profile is the tenant: one user's config, credential, workspace and
//! sandbox, laid out like the desktop's user directory
//! (`<root>/users/<profile-id>/`, the shared
//! [`ProfileLayout`](crate::config::schema::ProfileLayout)). The gateway
//! authenticates users; this domain turns a gateway user id into the
//! profile that serves that user ([`ProfileId`], per
//! [`ProfileIdMode`]), lays out its private state ([`layout`]), forces the
//! config it runs with ([`layout::profile_config`]), and keeps the open
//! profiles of the process ([`ProfileHost`]). The operator plane provisions
//! and inspects them through the `profiles.*` controllers ([`schemas`]).
//!
//! The isolation boundary is the profile's own [`CoreContext`]: its config,
//! workspace and `profile` (the tenant key, set to the profile id); its
//! default agent runs with no `session_agent`, as on the desktop. Work for
//! one user runs under that context, which is what the
//! config loader, the session store and the per-thread caches key on.
//!
//! One process hosts a profile at a time: opening one takes its lease
//! ([`lease`]), and the provisioned set lives in the [`registry`] — the
//! shared storage backend when one is configured, else `profile.toml` files.
//!
//! [`CoreContext`]: crate::core::runtime::CoreContext

pub mod background;
pub mod capabilities;
pub mod credentials;
pub mod cron;
pub mod fence;
pub mod gateway;
pub mod host;
pub mod layout;
pub mod lease;
pub mod lifecycle;
pub mod ops;
mod recovery;
pub mod registry;
mod storage_schema;
pub use storage_schema::storage_schema;
pub mod schemas;
pub mod surface;
pub mod tools;
pub mod turn_gate;
pub mod types;

pub use host::{current, Profile, ProfileHost};
pub use layout::ProfileLayout;
pub use lease::OpenError;
pub use registry::ProfileRegistry;
pub use schemas::{all_profiles_controller_schemas, all_profiles_registered_controllers};
pub use types::{ProfileId, ProfileIdMode, ProfileSummary};

#[doc(hidden)]
pub mod provider_settings;
