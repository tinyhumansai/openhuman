//! Privacy-mode enforcement at the egress chokepoints, decided by TinySecurity.
//!
//! Every external transfer builds an [`EgressDescriptor`] (what leaves, to where,
//! why). Before a site discloses and sends, it asks whether the registered privacy
//! mode permits the transfer. The module answers (`PrivacyCheck2`); this file only
//! translates the descriptor into the module's question and the answer into the
//! host's refusal text. It owns no truth table.
//!
//! * Every mode asks the module. `Standard` adds no privacy restriction, but a
//!   missing, faulted or slow module still **refuses** the transfer; the host
//!   never assumes an external effect is permitted.
//! * What the host contributes is the one thing only it knows: whether an
//!   integration descriptor represents a control-plane round trip
//!   ([`data_class`]). The TinyHumans adapter classifies its backend routes and
//!   carries that fact on the descriptor; core does not parse backend paths.
//!
//! The mode is the live session policy's
//! ([`current_privacy_mode`](crate::security::live_policy::current_privacy_mode));
//! it defaults to `Standard` when no session policy is installed (CLI, cron,
//! background); the module still owns the decision for those external effects.

use super::types::{DataKind, EgressDescriptor, EgressReason, IdentificationRisk};
use crate::config::PrivacyMode;
use crate::security::live_policy::current_privacy_mode;
use tinytools::status::POLICY_BLOCKED_MARKER;

/// What a transfer carries, coarsely. The module's `PrivacyDataClass2`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrivacyClass {
    /// Session, auth, team, billing and connection-management round trips.
    ControlPlane,
    /// Ids, counts and routing fields.
    Metadata,
    /// Prompts, tool arguments, embedding input, file content, destinations.
    UserContent,
}

/// The class of the transfer `desc` describes.
///
/// Only integration descriptors carrying the TinyHumans adapter's explicit
/// control-plane classification receive that class. Unclassified integrations
/// default to metadata or user content based on their declared data kinds.
#[must_use]
pub fn data_class(desc: &EgressDescriptor) -> PrivacyClass {
    if desc.reason == EgressReason::Integration && desc.control_plane {
        PrivacyClass::ControlPlane
    } else if desc
        .data_kinds
        .iter()
        .all(|kind| *kind == DataKind::Metadata)
    {
        PrivacyClass::Metadata
    } else {
        PrivacyClass::UserContent
    }
}

/// The assessed identification risk of the payload, if there is one. `Unknown`
/// means it was not assessed.
fn assessed_risk(desc: &EgressDescriptor) -> Option<(IdentificationRisk, &[String])> {
    match desc.risk_level {
        IdentificationRisk::Unknown => None,
        level => Some((level, desc.risk_categories.as_slice())),
    }
}

/// The module's verdict on one transfer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Permitted,
    Refused,
    /// The module could not answer. Refused, with a different message.
    Unavailable,
}

/// Human-readable, non-sensitive reason the transfer was blocked. Names only the
/// destination *service* (already a coarse slug, endpoint or host on the
/// descriptor, never a raw payload) and how to lift the block. No marker; callers
/// that need the [`POLICY_BLOCKED_MARKER`] prefix add it.
fn block_message(mode: PrivacyMode, desc: &EgressDescriptor, outcome: Outcome) -> String {
    if outcome == Outcome::Unavailable {
        return format!(
            "Privacy mode is active and the security module could not confirm that sending to \
             external service `{}` is allowed, so the action was refused.",
            desc.service
        );
    }
    if mode == PrivacyMode::Sensitive {
        return format!(
            "Sensitive privacy mode is active: this action would send content to external \
             service `{}` without a clean identification assessment. Change the privacy \
             mode in Settings to allow it.",
            desc.service
        );
    }
    format!(
        "Local-only privacy mode is active: this action needs external service \
         `{}`. Disable local-only mode in Settings to allow it.",
        desc.service
    )
}

/// The refusal for `desc` under `mode`, or `None` when permitted.
pub async fn egress_blocks_message(mode: PrivacyMode, desc: &EgressDescriptor) -> Option<String> {
    let outcome = native::decide(mode, desc).await;
    refusal(mode, desc, outcome)
}

/// [`egress_blocks_message`] for a synchronous caller.
pub fn egress_blocks_message_blocking(
    mode: PrivacyMode,
    desc: &EgressDescriptor,
) -> Option<String> {
    let outcome = native::decide_blocking(mode, desc);
    refusal(mode, desc, outcome)
}

fn refusal(mode: PrivacyMode, desc: &EgressDescriptor, outcome: Outcome) -> Option<String> {
    if outcome == Outcome::Permitted {
        log::debug!(
            "[privacy][egress-enforce] privacy_mode={mode:?} provider={} service={} reason={:?} — permitted",
            desc.provider_slug,
            desc.service,
            desc.reason,
        );
        return None;
    }
    log::warn!(
        "[privacy][egress-enforce] {mode:?} BLOCK ({outcome:?}) provider={} service={} reason={:?} — refused",
        desc.provider_slug,
        desc.service,
        desc.reason,
    );
    Some(block_message(mode, desc, outcome))
}

/// Enforce the live privacy policy for an `anyhow`-returning egress site (composio
/// tool calls, backend integrations, cloud embeddings, media generation). `Err`
/// carries a clean, non-sensitive message. Call it **before** the
/// disclose-and-send so a blocked transfer is neither disclosed as pending nor
/// dispatched.
///
/// # Errors
/// The transfer is refused by the registered privacy mode, or the module could not
/// answer under a restrictive mode.
pub async fn enforce_egress(desc: &EgressDescriptor) -> anyhow::Result<()> {
    match egress_blocks_message(current_privacy_mode(), desc).await {
        Some(message) => anyhow::bail!("{message}"),
        None => Ok(()),
    }
}

/// Synchronous counterpart for owner contracts whose guard callbacks cannot await.
///
/// # Errors
/// The transfer is refused by the registered privacy mode, or the module could not
/// answer under a restrictive mode.
pub fn enforce_egress_blocking(desc: &EgressDescriptor) -> anyhow::Result<()> {
    match egress_blocks_message_blocking(current_privacy_mode(), desc) {
        Some(message) => anyhow::bail!("{message}"),
        None => Ok(()),
    }
}

/// Enforce the live privacy policy for an agent tool whose `execute` returns
/// `Ok(ToolResult::error(..))` on a denied action. The message carries
/// [`POLICY_BLOCKED_MARKER`] so the agent loop treats it as a hard cross-turn block.
pub async fn local_only_tool_block(desc: &EgressDescriptor) -> Option<String> {
    egress_blocks_message(current_privacy_mode(), desc)
        .await
        .map(|message| format!("{POLICY_BLOCKED_MARKER} {message}"))
}

/// [`local_only_tool_block`] for a synchronous caller (the tool traits).
pub fn local_only_tool_block_blocking(desc: &EgressDescriptor) -> Option<String> {
    egress_blocks_message_blocking(current_privacy_mode(), desc)
        .map(|message| format!("{POLICY_BLOCKED_MARKER} {message}"))
}

#[cfg(feature = "security-module")]
mod native {
    use super::*;
    use crate::modules::security::gap;
    use tinysecurity_bus::{
        PrivacyCheckRequest2, PrivacyDataClass2, PrivacyMode as BusMode, PrivacyRisk2,
        PrivacyRiskLevel2, Verdict,
    };

    fn request(mode: PrivacyMode, desc: &EgressDescriptor) -> PrivacyCheckRequest2 {
        PrivacyCheckRequest2 {
            privacy: match mode {
                PrivacyMode::Standard => BusMode::Standard,
                PrivacyMode::Sensitive => BusMode::Sensitive,
                PrivacyMode::LocalOnly => BusMode::LocalOnly,
            },
            external: desc.is_external,
            class: match data_class(desc) {
                PrivacyClass::ControlPlane => PrivacyDataClass2::ControlPlane,
                PrivacyClass::Metadata => PrivacyDataClass2::Metadata,
                PrivacyClass::UserContent => PrivacyDataClass2::UserContent,
            },
            destination_label: desc.provider_slug.chars().take(128).collect(),
            risk: assessed_risk(desc).map(|(level, categories)| PrivacyRisk2 {
                level: match level {
                    IdentificationRisk::Low => PrivacyRiskLevel2::Low,
                    IdentificationRisk::Medium => PrivacyRiskLevel2::Medium,
                    IdentificationRisk::High => PrivacyRiskLevel2::High,
                    // `Unknown` never reaches here (`assessed_risk` filters it).
                    IdentificationRisk::None | IdentificationRisk::Unknown => {
                        PrivacyRiskLevel2::None
                    }
                },
                categories: categories
                    .iter()
                    .take(tinysecurity_bus::MAX_PRIVACY_CATEGORIES)
                    .map(|category| category.chars().take(64).collect())
                    .collect(),
            }),
        }
    }

    fn outcome(answer: Result<tinysecurity_bus::Decision, String>) -> Outcome {
        match answer {
            Ok(decision) => match decision.verdict {
                Verdict::Allow => Outcome::Permitted,
                _ => Outcome::Refused,
            },
            Err(error) => {
                log::warn!("[privacy][egress-enforce] module unavailable: {error}");
                Outcome::Unavailable
            }
        }
    }

    pub(super) async fn decide(mode: PrivacyMode, desc: &EgressDescriptor) -> Outcome {
        outcome(gap::privacy_check(request(mode, desc)).await)
    }

    pub(super) fn decide_blocking(mode: PrivacyMode, desc: &EgressDescriptor) -> Outcome {
        let request = request(mode, desc);
        #[cfg(test)]
        {
            return outcome(futures::executor::block_on(gap::privacy_check(request)));
        }
        #[cfg(not(test))]
        {
            outcome(crate::modules::security::text::privacy_check_blocking(
                request,
                std::time::Duration::from_secs(2),
            ))
        }
    }
}

#[cfg(not(feature = "security-module"))]
mod native {
    use super::*;

    pub(super) async fn decide(_mode: PrivacyMode, _desc: &EgressDescriptor) -> Outcome {
        Outcome::Unavailable
    }

    pub(super) fn decide_blocking(_mode: PrivacyMode, _desc: &EgressDescriptor) -> Outcome {
        Outcome::Unavailable
    }
}

#[cfg(test)]
#[path = "privacy_tests.rs"]
mod tests;
