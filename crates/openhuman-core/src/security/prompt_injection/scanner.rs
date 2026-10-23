//! The prompt scanner port.
//!
//! Which text is an injection attempt is the security module's decision
//! (`ScanPrompt2`, `ScanToolDefinition`); this file is how the host asks. The
//! hot path screens every tool output and every web and stored piece of content,
//! so the call is bounded: each module call has an explicit deadline, long text is
//! scanned in overlapping windows the module accepts, and anything the host could
//! not get an answer for is held for review, never waved through.
//!
//! [`PromptScanner`] is the seam. Production asks the module
//! ([`super::module_scanner`]); a build without the module asks nothing and holds
//! everything; tests use a task-scoped stub without task-local state in the
//! production scanner.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

/// How much friction a prompt deserves, from the module's tiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    /// Nothing worth friction.
    Allow,
    /// Suspicious; the host holds it.
    Review,
    /// A strong injection signal; the host refuses it.
    Block,
}

/// What the module made of one text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptScan {
    pub tier: Tier,
    /// Additive score in thousandths, at most 1000.
    pub score_milli: u16,
    /// Content-free rule codes, in detection order.
    pub rule_ids: Vec<String>,
}

impl PromptScan {
    /// Clean: nothing fired.
    pub fn clean() -> Self {
        Self {
            tier: Tier::Allow,
            score_milli: 0,
            rule_ids: Vec::new(),
        }
    }

    /// The text could not be screened, so it is held for review.
    pub fn held(rule: &str) -> Self {
        Self {
            tier: Tier::Review,
            score_milli: REVIEW_FLOOR_MILLI,
            rule_ids: vec![rule.to_owned()],
        }
    }

    fn merge(&mut self, other: Self) {
        self.tier = self.tier.max(other.tier);
        self.score_milli = self.score_milli.max(other.score_milli);
        for id in other.rule_ids {
            if !self.rule_ids.contains(&id) {
                self.rule_ids.push(id);
            }
        }
    }
}

/// The module's review threshold, in thousandths: the score a held text carries.
pub const REVIEW_FLOOR_MILLI: u16 = 550;

/// The module's verdict on a tool definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefinitionVerdict {
    Clean,
    Suspicious,
    Blocked,
}

/// What the module made of one tool definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinitionScan {
    pub verdict: DefinitionVerdict,
    pub rule_ids: Vec<String>,
}

/// Where the host asks whether text or a tool definition is an injection attempt.
#[async_trait]
pub trait PromptScanner: Send + Sync + std::fmt::Debug {
    /// Score one text no longer than [`WINDOW_BYTES`] plus its overlap.
    async fn scan_prompt(&self, text: &str) -> Result<PromptScan, String>;

    /// Scan one tool definition: its name and a JSON document of its fields.
    async fn scan_tool_definition(
        &self,
        name: &str,
        definition: &serde_json::Value,
    ) -> Result<DefinitionScan, String>;
}

/// Longest slice sent in one call; the module accepts 64 KiB.
pub const WINDOW_BYTES: usize = 48_000;
/// Bytes two neighbouring windows share, so a phrase on a boundary is whole in one.
pub const OVERLAP_BYTES: usize = 4_096;
/// Most text screened per call; more is held rather than scanned in part.
pub const MAX_SCANNED_BYTES: usize = 8 * 1024 * 1024;
/// How long one module call may take before the text is held.
pub const CALL_DEADLINE: Duration = Duration::from_secs(2);
/// How long the whole screen of one text may take before it is held.
pub const TOTAL_DEADLINE: Duration = Duration::from_secs(10);

/// The production scanner is always the TinySecurity module.
pub(super) fn current() -> Arc<dyn PromptScanner> {
    #[cfg(test)]
    if let Some(scanner) = tests::current_scoped() {
        return scanner;
    }
    super::module_scanner::production()
}

/// The slices of `text` to scan: one when it fits, otherwise overlapping windows
/// cut on character boundaries. `None` when it is longer than
/// [`MAX_SCANNED_BYTES`].
pub fn windows(text: &str) -> Option<Vec<&str>> {
    if text.len() > MAX_SCANNED_BYTES {
        return None;
    }
    if text.len() <= WINDOW_BYTES {
        return Some(vec![text]);
    }
    let mut out = Vec::with_capacity(text.len() / (WINDOW_BYTES - OVERLAP_BYTES) + 1);
    let mut start = 0;
    while start < text.len() {
        let mut end = (start + WINDOW_BYTES).min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        out.push(&text[start..end]);
        if end == text.len() {
            break;
        }
        let mut next = end - OVERLAP_BYTES;
        while !text.is_char_boundary(next) {
            next += 1;
        }
        start = next;
    }
    Some(out)
}

/// Screen `text` through the scanner in force. Never errors: a text the host
/// could not get an answer for comes back held for review.
pub async fn screen(text: &str) -> PromptScan {
    let Some(slices) = windows(text) else {
        tracing::warn!(
            bytes = text.len(),
            "[prompt_injection] text too large to screen; holding it for review"
        );
        return PromptScan::held("screen.too_large");
    };
    let scanner = current();
    let all = async {
        let mut total = PromptScan::clean();
        for slice in slices {
            match tokio::time::timeout(CALL_DEADLINE, scanner.scan_prompt(slice)).await {
                Ok(Ok(scan)) => {
                    let blocked = scan.tier == Tier::Block;
                    total.merge(scan);
                    if blocked {
                        break;
                    }
                }
                Ok(Err(error)) => {
                    tracing::warn!(
                        error = %error,
                        "[prompt_injection] the scanner could not answer; holding the text for review"
                    );
                    total.merge(PromptScan::held("screen.unavailable"));
                    break;
                }
                Err(_) => {
                    tracing::warn!(
                        "[prompt_injection] the scanner timed out; holding the text for review"
                    );
                    total.merge(PromptScan::held("screen.timeout"));
                    break;
                }
            }
        }
        total
    };
    tokio::time::timeout(TOTAL_DEADLINE, all)
        .await
        .unwrap_or_else(|_| PromptScan::held("screen.timeout"))
}

/// Scan a tool definition through the scanner in force. `Err` when the scanner
/// could not answer within [`CALL_DEADLINE`]; a caller admitting remote tools
/// treats that as a hit.
pub async fn screen_tool_definition(
    name: &str,
    definition: &serde_json::Value,
) -> Result<DefinitionScan, String> {
    let scanner = current();
    match tokio::time::timeout(
        CALL_DEADLINE,
        scanner.scan_tool_definition(name, definition),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => Err("the tool definition scan timed out".into()),
    }
}

#[cfg(test)]
#[path = "scanner_tests.rs"]
pub(crate) mod tests;
