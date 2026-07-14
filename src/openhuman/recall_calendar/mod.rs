//! Recall.ai Calendar V1 integration (backend-proxied).
//!
//! A less-invasive replacement for Composio-based Google Calendar sync as the
//! Google Meet detection source. The user connects their Google Calendar once
//! via Recall's hosted OAuth (only `calendar.events.readonly` + `userinfo.email`
//! scopes); the core reads upcoming meetings through the openhuman backend and
//! feeds them into the existing meeting auto-join path. Bots are still scheduled
//! by us (credit-gated, per-user mascot), not by Recall.
//!
//! Selected via `config.meet.calendar_provider == Recall`.

pub mod ops;
pub mod schemas;
pub mod types;

pub use schemas::{
    all_controller_schemas as all_recall_calendar_controller_schemas,
    all_registered_controllers as all_recall_calendar_registered_controllers,
};
