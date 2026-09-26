//! Backend-managed webhook tunnels (`/webhooks/core*`): tunnel CRUD and the
//! remaining bandwidth budget, on the SDK's typed `webhooks()` client.
//!
//! The RPC names (`openhuman.webhooks_{list,create,get,update,delete}_tunnel`,
//! `openhuman.webhooks_get_bandwidth`) are unchanged wire contracts. They share
//! the `webhooks` namespace with the core's local webhook router controllers
//! (`list_registrations`, `register_echo`, `trigger_agent`, …), which stay in
//! the core because they route inbound deliveries to skills and agents.

mod ops;
mod schemas;

pub use ops::*;
pub use schemas::{
    all_webhooks_controller_schemas, all_webhooks_registered_controllers, webhooks_schemas,
};
