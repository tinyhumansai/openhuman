//! Agent integrations that run through the core itself.
//!
//! Composio connectors ([`composio`]) and task sources ([`task_sources`]) live
//! here. The hosted integrations client, its managed tools (Google
//! Places, stock data, file storage, media generation) and the web3 crypto
//! client are not part of the core: a host that has such a service plugs them
//! in through the generic registries (tool sources, the Composio route
//! builders, the wallet-backend hook). With none registered the core runs
//! without them.

pub mod composio;
pub mod task_sources;
