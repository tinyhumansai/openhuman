//! Whether memory work is free for the user right now.
//!
//! Background memory jobs that write a user's whole memory again (moving it
//! into a new layout, importing a local store) start on their own only when
//! that costs the user nothing; otherwise they wait for the user to start them.
//! [`free_period_active`] is the one check every such job asks:
//!
//! - **An engine with a registered predicate**
//!   ([`engine_registry`](super::engine_registry)): the predicate decides. A
//!   registration without one is free.
//! - **Any other engine** (self-hosted CortexDB, or a host's own engine):
//!   always free here, because no hosted credit is spent.
//! - **Anything unknown** (memory off, signed out, an engine that failed to
//!   bind): not free, so nothing starts on its own.

use crate::config::Config;

use super::engine::{self, Binding};

/// Whether memory work is free for the user right now. See the module docs.
pub async fn free_period_active(config: &Config) -> bool {
    let bound = match engine::resolve(config) {
        Binding::On(bound) => bound,
        Binding::Off { .. } => return false,
    };
    if let Some(entry) = super::engine_registry::lookup(&bound.id) {
        return match entry.free_background {
            Some(free) => free(config.clone(), bound).await,
            None => true,
        };
    }
    true
}

#[cfg(test)]
#[path = "billing_tests.rs"]
mod tests;
