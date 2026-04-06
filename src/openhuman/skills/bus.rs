//! Event bus handlers for the skills domain.
//!
//! Placeholder for future cross-domain subscribers that react to skill lifecycle
//! events (e.g. cascading restarts, dependency tracking, metrics collection).
//!
//! Skill events are currently consumed by the [`TracingSubscriber`] for
//! observability. Add domain-specific handlers here as needed following the
//! pattern in `crate::openhuman::cron::bus`.
