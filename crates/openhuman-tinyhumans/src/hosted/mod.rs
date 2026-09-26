//! Clients of the hosted TinyHumans backend.
//!
//! Every domain here is a thin proxy to `tinyhumansai/backend` — the truth lives
//! server-side and this side only authenticates, forwards, and shapes results.
//! They live in this crate, not the core, because a core without a TinyHumans
//! connection has no use for them: [`extension`] hands them to the core's
//! controller registry (`DomainGroup::Hosted`) when [`crate::install`] runs,
//! and the RPC names (`openhuman.billing_*`, `team_*`, `referral_*`,
//! `announcements_*`, and the account-bound `auth_*` / `channels_*` /
//! `webhooks_*` methods below) are unchanged wire contracts.
//!
//! Every domain reaches the backend through [`client::HostedClient`]: the
//! SDK's typed client, authenticated with the core's resolved credential, with
//! one mapping from SDK errors to the core's RPC sentinels. A user with no
//! TinyHumans account (offline local session, signed out) gets the core's
//! `BACKEND_UNAVAILABLE:` / session sentinel without a request.
//!
//! - [`announcements`] — product announcements feed
//! - [`billing`]       — credits, plans, Stripe/Coinbase-backed balance reads
//! - [`channel_link`]  — managed Telegram/Discord bot linking
//!   (`auth.create_channel_link_token`, `channels.telegram_login_*`,
//!   `channels.discord_link_*`)
//! - [`oauth`]         — backend-brokered OAuth integrations (`auth.oauth_*`)
//! - [`referral`]      — referral codes and rewards
//! - [`team`]          — team membership/roles/invites and the usage document
//!   (authorization is enforced server-side; this is a proxy)
//! - [`webhooks`]      — backend-managed webhook tunnels and bandwidth
//!
//! `channel_link`, `oauth` and `webhooks` share the `auth`, `channels` and
//! `webhooks` namespaces with core controllers. The registry keys controllers
//! by `namespace.function`, so a namespace may be split between the core and
//! this extension; the core keeps describing those namespaces, which is why
//! they are not in [`NAMESPACES`].

pub mod announcements;
pub mod billing;
pub mod channel_link;
pub mod client;
pub mod oauth;
pub mod referral;
pub mod team;
pub mod webhooks;

#[cfg(test)]
pub(crate) mod test_support;

use openhuman_core::core::all::{ControllerExtension, DomainGroup};

/// Namespace descriptions the core's `namespace_description` serves for the
/// namespaces only the hosted surface uses. Shared namespaces (`auth`,
/// `channels`, `webhooks`) are described by the core.
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
    controllers.extend(webhooks::all_webhooks_registered_controllers());
    controllers.extend(channel_link::all_channel_link_registered_controllers());
    controllers.extend(oauth::all_oauth_registered_controllers());
    ControllerExtension {
        group: DomainGroup::Hosted,
        controllers,
        namespaces: NAMESPACES,
    }
}
