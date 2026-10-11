//! Channel definitions, connection management, and RPC controllers.

mod backend;
mod ops;
mod schemas;

pub use backend::OpenHumanChannelBackend;

pub use schemas::all_controller_schemas as all_channels_controller_schemas;

/// Every controller of the channels domain: the `channels.*` connection
/// management above, plus the hosted-channel relay
/// (`channel.relay_inbound`, `providers::relay`).
pub fn all_channels_registered_controllers() -> Vec<crate::core::all::RegisteredController> {
    let mut controllers = schemas::all_registered_controllers();
    controllers.extend(crate::channels::providers::relay::all_relay_registered_controllers());
    controllers
}

/// Cross-module helpers from the channel controller layer that callers
/// outside the controller registry need (e.g. the welcome agent's
/// onboarding status snapshot).
pub use ops::connected_channel_slugs;
