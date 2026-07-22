//! Disabled-build stub for the `desktop_companion` domain (`desktop-automation`
//! off). Supplies only the controller aggregators — the registration site in
//! `core::all` wants absence, so both return empty vecs. `types` and `bus` stay
//! compiled (dep-free), so the always-on `core::socketio` subscriber and any
//! type consumer need no stub.

use crate::core::all::RegisteredController;
use crate::core::ControllerSchema;

/// Real: `schemas::all_desktop_companion_registered_controllers`. Empty ⇒ the
/// `companion.*` controllers are unregistered (unknown-method over `/rpc`,
/// absent from `/schema`).
pub fn all_desktop_companion_registered_controllers() -> Vec<RegisteredController> {
    Vec::new()
}

/// Real: `schemas::all_desktop_companion_controller_schemas`.
pub fn all_desktop_companion_controller_schemas() -> Vec<ControllerSchema> {
    Vec::new()
}
