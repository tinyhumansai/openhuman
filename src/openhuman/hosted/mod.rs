//! Clients of the hosted TinyHumans backend.
//!
//! Every domain here is a thin proxy to `tinyhumansai/backend` — the truth lives
//! server-side and this side only authenticates, forwards, and shapes results.
//! A self-hosted or embedded build that never dials the managed backend drops
//! them together, which is why they are one family.
//!
//! - [`announcements`] — product announcements feed
//! - [`billing`]       — credits, plans, Stripe/Coinbase-backed balance reads
//! - [`orchestration`] — device-side client of the hosted orchestration brain
//! - [`referral`]      — referral codes and rewards
//! - [`team`]          — team membership/roles/invites (authorization is
//!   enforced server-side; this is a proxy, not a local implementation)
//!
//! **`orchestration` here is the hosted client, not the local control plane.**
//! The local one is `agent_orchestration` (moving to `agent/orchestration`).
//! The names rhyme; the two are unrelated and must not be merged.

pub mod announcements;
pub mod billing;
pub mod orchestration;
pub mod referral;
pub mod team;
