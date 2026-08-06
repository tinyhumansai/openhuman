//! Desktop-shell-facing surfaces.
//!
//! Domains that exist to serve the Tauri desktop client specifically — the
//! things a headless / embedded host has no use for. Grouped so a future
//! `desktop` gate can drop them as one unit.
//!
//! - [`accessibility`]     — OS accessibility APIs (screen reader, permissions,
//!   vision-assisted click). Reached today only from the `voice` family.
//! - [`app_state`]         — persisted desktop app state
//! - [`dashboard`]         — dashboard aggregation surface
//! - [`notifications`]     — user-facing notification delivery
//! - [`overlay`]           — the desktop overlay window surface
//! - [`provider_surfaces`] — per-provider UI surface descriptors
//!
//! Not yet gated: `accessibility`'s `cpal` microphone probe already rides the
//! `inference` gate, and the rest are ungated. See
//! `docs/specs/2026-08-02-core-kernel-domain-reorg.md`.

pub mod accessibility;
pub mod app_state;
pub mod dashboard;
pub mod notifications;
pub mod overlay;
pub mod provider_surfaces;
