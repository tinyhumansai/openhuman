//! Event-bus glue for `harness_init`.
//!
//! Progress events are published directly from [`super::store::update_step`],
//! so no subscriber is required today. This module is kept for canonical
//! module shape and as the home for any future health re-publishing (e.g.
//! mirroring spaCy/Node readiness into the `health` domain).
