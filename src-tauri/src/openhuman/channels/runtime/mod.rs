//! Channel runtime entry points.

mod dispatch;
mod startup;
mod supervision;

pub use startup::start_channels;

pub(crate) use dispatch::{process_channel_message, run_message_dispatch_loop};
pub(crate) use supervision::spawn_supervised_listener;
