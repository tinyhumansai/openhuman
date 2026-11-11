//! Cleanups of boxes the registry no longer tracks: the pending-cleanup
//! count per owner that makes a new box wait for an old one's close or
//! forget, and the detached box a cleanup task lets go of.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use super::registry::Owner;
use super::BoxDriver;

/// Cleanups (closes or forgets) still running, per owner. A new box for an
/// owner waits until none is pending, so it never adopts an instance a
/// cleanup is still tearing down.

/// Let go of `resource` through `driver`: forget it when `retained`, close it
/// otherwise. A failure is logged.
pub(super) async fn let_go_with<D: BoxDriver>(driver: &D, retained: bool, resource: &str) {
    let outcome = if retained {
        driver.forget(resource).await
    } else {
        driver.close(resource).await
    };
    if let Err(error) = outcome {
        tracing::warn!(
            resource = %resource,
            retained,
            error = %error,
            "[sandbox:box] letting go of a box failed"
        );
    }
}
