//! Domain types for tool-call lifecycle state and failure classification.
//!
//! These types are the shared vocabulary for the "Visible tool status, failure
//! diagnosis, and safe recovery flows" epic (#4254). They are intentionally
//! transport-agnostic serde types: the same values are carried on the
//! `tool` event-bus domain, returned over JSON-RPC, and rendered in the UI.
//!
//! Nothing here performs classification — see [`super::classify`] for the pure
//! mapping from a raw tool error into a [`ClassifiedFailure`]. Keeping the data
//! model and the heuristics in separate files lets the heuristics be unit
//! tested without any runtime state.

use serde::{Deserialize, Serialize};

/// Where a single tool call is in its lifecycle.
///
/// Rendered 1:1 in the UI status surfaces (chat timeline today, dedicated
/// status panel in a later phase). `Queued`/`Retrying`/`NeedsUserInput` are not
/// yet emitted by the executor in P1 — they are modelled here so downstream
/// phases (retry, status panel) do not need to reshape the enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolLifecycleState {
    /// Accepted but not yet started (e.g. waiting on a concurrency slot).
    Queued,
    /// Executing right now.
    Running,
    /// Finished successfully.
    Succeeded,
    /// Finished with a failure. Pair with a [`ClassifiedFailure`] for the cause.
    Failed,
    /// Refused before execution by the security policy / permission gate.
    Blocked,
    /// A bounded automatic retry is in progress.
    Retrying,
    /// Parked awaiting the user (approval or additional input).
    NeedsUserInput,
}

/// The kind of failure, one of the classes called out in #4254 plus a catch-all.
///
/// The set is deliberately small and user-facing — it maps to the plain-language
/// explanations in [`ClassifiedFailure`], not to internal error enums.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolFailureClass {
    /// OpenHuman lacks an OS/tool permission it needs (e.g. file access).
    MissingPermission,
    /// A required application or command is not installed / not available.
    MissingApp,
    /// A dependency service is temporarily unavailable / unreachable.
    ServiceUnavailable,
    /// Saved credentials are missing, expired, or invalid.
    BadCredentials,
    /// The action was refused by the user's safety / autonomy policy.
    BlockedByPolicy,
    /// The AI model / provider could not be reached.
    ModelConnection,
    /// The action ran past its deadline and was stopped.
    Timeout,
    /// The user (or the approval gate on their behalf) refused this action at
    /// the approval prompt. Non-retryable — re-running just re-prompts for an
    /// effect the user already declined (#4459).
    Denied,
    /// The approval prompt expired (TTL) before anyone responded. Non-retryable:
    /// nobody approved, so it must not read as an execution timeout that
    /// auto-retries (#4459).
    ApprovalExpired,
    /// Could not be classified into any of the above.
    Unknown,
}

/// The three top-level states the UI separates, per the #4254 acceptance
/// criterion "clear separation between recoverable failure, blocked-by-policy,
/// and action-needs-user-confirmation states".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FailureCategory {
    /// Transient — safe for OpenHuman to retry on its own (bounded, in a later
    /// phase). `recoverable` on [`ClassifiedFailure`] is true iff this variant.
    Recoverable,
    /// Refused by policy. Not retried; the user must change settings to allow it.
    BlockedByPolicy,
    /// Needs the user to act (grant permission, install an app, sign in) before
    /// the action can succeed.
    NeedsUserConfirmation,
    /// The user declined the action, or the approval prompt expired before
    /// anyone responded. Never retried automatically — auto-re-attempting a
    /// refused external effect is exactly the bug this category prevents
    /// (#4459).
    UserDeclined,
}

/// A tool failure rendered for a non-technical user: what class it is, which
/// top-level category it falls in, a plain-language cause, and a next action.
///
/// Constructed only via [`super::classify`]; the string fields are stable,
/// user-facing copy (English source; localized in the UI layer).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassifiedFailure {
    /// The specific failure class.
    pub class: ToolFailureClass,
    /// The top-level category the UI separates on.
    pub category: FailureCategory,
    /// Plain-language description of what went wrong (no jargon, no stack).
    pub cause_plain: String,
    /// Plain-language description of what to do next.
    pub next_action: String,
    /// Whether OpenHuman may retry automatically. Always equal to
    /// `category == FailureCategory::Recoverable` — carried explicitly so
    /// serialized consumers don't have to re-derive it.
    pub recoverable: bool,
}

impl FailureCategory {
    /// Whether a failure in this category is safe to retry automatically.
    pub fn is_recoverable(self) -> bool {
        matches!(self, FailureCategory::Recoverable)
    }
}

impl ToolFailureClass {
    /// The category this class belongs to. Single source of truth for the
    /// class→category mapping used by [`super::classify`].
    pub fn category(self) -> FailureCategory {
        match self {
            ToolFailureClass::ServiceUnavailable
            | ToolFailureClass::ModelConnection
            | ToolFailureClass::Timeout
            | ToolFailureClass::Unknown => FailureCategory::Recoverable,
            ToolFailureClass::BlockedByPolicy => FailureCategory::BlockedByPolicy,
            ToolFailureClass::Denied | ToolFailureClass::ApprovalExpired => {
                FailureCategory::UserDeclined
            }
            ToolFailureClass::MissingPermission
            | ToolFailureClass::MissingApp
            | ToolFailureClass::BadCredentials => FailureCategory::NeedsUserConfirmation,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recoverable_iff_recoverable_category() {
        assert!(FailureCategory::Recoverable.is_recoverable());
        assert!(!FailureCategory::BlockedByPolicy.is_recoverable());
        assert!(!FailureCategory::NeedsUserConfirmation.is_recoverable());
        assert!(!FailureCategory::UserDeclined.is_recoverable());
    }

    #[test]
    fn every_class_maps_to_expected_category() {
        use FailureCategory::*;
        use ToolFailureClass::*;
        assert_eq!(ServiceUnavailable.category(), Recoverable);
        assert_eq!(ModelConnection.category(), Recoverable);
        assert_eq!(Timeout.category(), Recoverable);
        assert_eq!(Unknown.category(), Recoverable);
        assert_eq!(
            ToolFailureClass::BlockedByPolicy.category(),
            FailureCategory::BlockedByPolicy
        );
        assert_eq!(MissingPermission.category(), NeedsUserConfirmation);
        assert_eq!(MissingApp.category(), NeedsUserConfirmation);
        assert_eq!(BadCredentials.category(), NeedsUserConfirmation);
        assert_eq!(Denied.category(), UserDeclined);
        assert_eq!(ApprovalExpired.category(), UserDeclined);
    }

    #[test]
    fn types_round_trip_through_json() {
        let f = ClassifiedFailure {
            class: ToolFailureClass::Timeout,
            category: FailureCategory::Recoverable,
            cause_plain: "took too long".into(),
            next_action: "try again".into(),
            recoverable: true,
        };
        let json = serde_json::to_string(&f).unwrap();
        let back: ClassifiedFailure = serde_json::from_str(&json).unwrap();
        assert_eq!(f, back);
    }

    #[test]
    fn lifecycle_state_serializes_to_variant_name() {
        let json = serde_json::to_string(&ToolLifecycleState::NeedsUserInput).unwrap();
        assert_eq!(json, "\"NeedsUserInput\"");
    }
}
