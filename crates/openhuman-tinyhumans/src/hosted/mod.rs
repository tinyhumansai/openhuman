//! Clients of the hosted TinyHumans backend.
//!
//! Every domain here is a thin proxy to `tinyhumansai/backend` — the truth lives
//! server-side and this side only authenticates, forwards, and shapes results.
//! They live in this crate, not the core, because a core without a TinyHumans
//! connection has no use for them: [`extension`] hands them to the core's
//! controller registry (`DomainGroup::Hosted`) when [`crate::install`] runs,
//! and the RPC names (`openhuman.billing_*`, `team_*`, `referral_*`,
//! `announcements_*`) are unchanged wire contracts.
//!
//! - [`announcements`] — product announcements feed
//! - [`billing`]       — credits, plans, Stripe/Coinbase-backed balance reads
//! - [`referral`]      — referral codes and rewards
//! - [`team`]          — team membership/roles/invites (authorization is
//!   enforced server-side; this is a proxy, not a local implementation)

pub mod announcements;
pub mod billing;
pub mod referral;
pub mod team;

use openhuman_core::core::all::{ControllerExtension, DomainGroup};

/// Namespace descriptions the core's `namespace_description` serves for the
/// hosted RPC surface.
pub const NAMESPACES: &[(&str, &str)] = &[
    (
        "billing",
        "Subscription plan, payment links, and credit top-up via the backend.",
    ),
    (
        "team",
        "Team member management, invites, and role changes via the backend.",
    ),
    (
        "referral",
        "Referral codes, stats, and apply flows via the hosted backend API.",
    ),
    (
        "announcements",
        "Latest active product announcement surfaced on harness init, via the backend.",
    ),
];

/// The hosted controllers as one registry extension.
pub fn extension() -> ControllerExtension {
    let mut controllers = referral::all_referral_registered_controllers();
    controllers.extend(billing::all_billing_registered_controllers());
    controllers.extend(announcements::all_announcements_registered_controllers());
    controllers.extend(team::all_team_registered_controllers());
    ControllerExtension {
        group: DomainGroup::Hosted,
        controllers,
        namespaces: NAMESPACES,
    }
}
