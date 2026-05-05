//! Channel definitions, connection management, and RPC controllers.

mod definitions;
mod ops;
mod schemas;

pub use definitions::{
    all_channel_definitions, find_channel_definition, AuthModeSpec, ChannelAuthMode,
    ChannelCapability, ChannelDefinition, FieldRequirement,
};

pub use schemas::{
    all_controller_schemas as all_channels_controller_schemas,
    all_registered_controllers as all_channels_registered_controllers,
};

/// Cross-module helpers from the channel controller layer that callers
/// outside the controller registry need (e.g. the welcome agent's
/// onboarding status snapshot).
pub use ops::connected_channel_slugs;
