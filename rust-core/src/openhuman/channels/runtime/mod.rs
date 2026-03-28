//! Channel runtime entry points.

mod dispatch;
mod startup;
mod supervision;

pub use startup::start_channels;

// Re-exported for `channels::tests` only; omit in normal lib builds to avoid unused-import warnings.
#[cfg(test)]
pub(crate) use dispatch::{process_channel_message, run_message_dispatch_loop};
#[cfg(test)]
pub(crate) use supervision::spawn_supervised_listener;
