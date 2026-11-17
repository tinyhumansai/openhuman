//! Desktop-shell-facing surfaces.
//!
//! Domains that exist to serve the Tauri desktop client specifically — the
//! things a headless / embedded host has no use for. Grouped so a future
//! `desktop` gate can drop them as one unit.
//!
//! - [`app_state`]         — persisted desktop app state
//! - [`dashboard`]         — dashboard aggregation surface
//! - [`notifications`]     — user-facing notification delivery
//! - [`provider_surfaces`] — per-provider UI surface descriptors
//!
//! OS accessibility (focus, permissions, paste and Globe listeners) executes
//! inside TinyComputer through its minimal bus contract. The modules below
//! remain ungated. See `docs/specs/2026-08-02-core-kernel-domain-reorg.md`.

pub mod app_state;
#[cfg(feature = "modules")]
pub mod control;
pub mod dashboard;
pub mod notifications;
pub mod provider_surfaces;
