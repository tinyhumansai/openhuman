//! OS-facing pieces of the OpenHuman core that need no product configuration.
//!
//! - [`service`]: install and drive the core as a per-user OS service
//!   (launchd, systemd user unit, scheduled task, or the E2E mock), described
//!   by a [`service::ServiceSpec`].
//! - [`update`]: find the newest core release on GitHub and stage its binary,
//!   addressed by an [`update::UpdateSource`], reporting failures through an
//!   [`update::UpdateReporter`].
//!
//! The core owns everything product-shaped around these: the controllers and
//! RPC schemas, restart and shutdown, update policy and the download
//! allowlist, the scheduler and the agent tools.

pub mod service;
pub mod update;

#[cfg(test)]
mod test_env;
