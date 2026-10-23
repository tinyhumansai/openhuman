//! Identification-risk assessment of the real outgoing payload (privacy epic
//! S5, #4439), answered by the TinySecurity module.
//!
//! The descriptor an egress site builds names the destination; it cannot know
//! what the payload contains until the payload exists. This module is the one
//! place a site that holds the payload asks the module (`Redact`, `Text` mode)
//! whether the shared redactor would change it, the same test the module's own
//! Sensitive-mode egress rule applies, and records the answer on the descriptor
//! with [`EgressDescriptor::with_risk`].
//!
//! * Only [`PrivacyMode::Sensitive`] depends on the answer, so under any other
//!   mode the payload is not sent to the module at all and the descriptor keeps
//!   [`IdentificationRisk::Unknown`] ("not assessed"). That keeps a module round
//!   trip off every inference, embedding and backend call in the default mode.
//! * An assessment the module cannot give (not armed yet, faulted, timed out,
//!   built without `security-module`) is [`IdentificationRisk::Unknown`], which
//!   Sensitive refuses. It is never assumed clean.
//! * A payload the redactor would change is [`IdentificationRisk::High`] with
//!   the category [`CATEGORY_SENSITIVE_CONTENT`]. The redactor does not say
//!   whether a hit was a credential or an identifier, and Sensitive refuses
//!   both, so the category names both.

use super::types::{EgressDescriptor, IdentificationRisk};
use crate::config::PrivacyMode;
use crate::security::live_policy::current_privacy_mode;

/// Category recorded when the redactor would change the payload. Contains the
/// `credential` and `identifier` needles the module's `PrivacyCheck2` Sensitive rule
/// refuses.
pub const CATEGORY_SENSITIVE_CONTENT: &str = "credential_or_identifier";

/// Longest the assessment waits for the module.
const ASSESS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// The result of assessing one payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assessment {
    pub level: IdentificationRisk,
    pub categories: Vec<String>,
}

impl Assessment {
    fn unassessed() -> Self {
        Self {
            level: IdentificationRisk::Unknown,
            categories: Vec::new(),
        }
    }
}

/// Pure mapping from the module's answer to an assessment.
///
/// `answer` is the redacted form of `original`, or why there is none. An
/// unchanged payload is clean; a changed one carries identifying content; no
/// answer is unassessed.
#[must_use]
pub fn assessment_from_answer(original: &str, answer: Result<String, String>) -> Assessment {
    match answer {
        Ok(redacted) if redacted == original => Assessment {
            level: IdentificationRisk::None,
            categories: Vec::new(),
        },
        Ok(_) => Assessment {
            level: IdentificationRisk::High,
            categories: vec![CATEGORY_SENSITIVE_CONTENT.to_string()],
        },
        Err(error) => {
            log::debug!("[privacy][egress-assess] module cannot assess; Unknown: {error}");
            Assessment::unassessed()
        }
    }
}

#[cfg(feature = "security-module")]
fn module_answer(text: &str) -> Result<String, String> {
    use crate::modules::security::text::{try_redact, RedactMode};
    try_redact(RedactMode::Text, text, Some(ASSESS_TIMEOUT))
}

#[cfg(not(feature = "security-module"))]
fn module_answer(_text: &str) -> Result<String, String> {
    Err("built without security-module".to_string())
}

/// Assess `payload` for `mode`: the module's answer under Sensitive, otherwise
/// "not assessed" without a module call.
#[must_use]
pub fn assess_for_mode(mode: PrivacyMode, payload: &str) -> Assessment {
    if mode != PrivacyMode::Sensitive {
        return Assessment::unassessed();
    }
    assessment_from_answer(payload, module_answer(payload))
}

/// Assess `payload` under the live privacy mode.
#[must_use]
pub fn assess_payload(payload: &str) -> Assessment {
    assess_for_mode(current_privacy_mode(), payload)
}

impl EgressDescriptor {
    /// Record the identification risk of the real outgoing `payload` (see the
    /// [module docs](self)). The payload itself is never stored.
    #[must_use]
    pub fn assessed(self, payload: &str) -> Self {
        let assessment = assess_payload(payload);
        if assessment.level == IdentificationRisk::Unknown {
            return self;
        }
        self.with_risk(assessment.level, assessment.categories)
    }

    /// As [`Self::assessed`] for several payload parts (a prompt's messages, a
    /// batch of embedding inputs): the worst part decides.
    #[must_use]
    pub fn assessed_parts<'a>(self, parts: impl IntoIterator<Item = &'a str>) -> Self {
        let mode = current_privacy_mode();
        if mode != PrivacyMode::Sensitive {
            return self;
        }
        let worst = parts
            .into_iter()
            .map(|part| assess_for_mode(mode, part))
            .max_by_key(|a| rank(a.level))
            .unwrap_or_else(|| Assessment {
                level: IdentificationRisk::None,
                categories: Vec::new(),
            });
        if worst.level == IdentificationRisk::Unknown {
            return self;
        }
        self.with_risk(worst.level, worst.categories)
    }
}

/// Worst-first ordering where an unassessed part outranks everything: one part
/// the module could not assess makes the whole batch unassessed.
fn rank(level: IdentificationRisk) -> u8 {
    match level {
        IdentificationRisk::Unknown => 5,
        IdentificationRisk::High => 4,
        IdentificationRisk::Medium => 3,
        IdentificationRisk::Low => 2,
        IdentificationRisk::None => 1,
    }
}

#[cfg(test)]
#[path = "assess_tests.rs"]
mod tests;
