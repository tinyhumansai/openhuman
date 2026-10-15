//! Module-backed prompt scan, layered on the host detector.
//!
//! The tinysecurity module's `ScanPrompt` is a second, content-free opinion on a
//! prompt. Its verdict is **context, not authorization**: `Clean` never
//! overrides a host block, `Suspicious` is logged for correlation, and only
//! `Blocked` adds friction (the prompt is refused). An unavailable module yields
//! `None` — the host detector remains in force and nothing is granted.

/// Module verdict, mirrored so callers compile without the module feature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleVerdict {
    Clean,
    Suspicious,
    Blocked,
}

/// What the module said about a prompt, content-free.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleScan {
    pub verdict: ModuleVerdict,
    pub rule_ids: Vec<String>,
}

impl ModuleScan {
    /// Only a `Blocked` verdict refuses the prompt.
    #[must_use]
    pub fn blocks(&self) -> bool {
        self.verdict == ModuleVerdict::Blocked
    }
}

/// Scan `text` through the module under the current dispatch scope. `None` when
/// the module is disabled, unavailable or no scope is established.
#[cfg(feature = "security-module")]
pub async fn module_scan(text: &str) -> Option<ModuleScan> {
    let config = crate::core::runtime::CoreContext::current_embedder_config()?;
    if !config.modules.enabled {
        return None;
    }
    match crate::modules::security::text::scan_prompt(&config, text).await {
        Ok(response) => {
            log::debug!(
                "[security][prompt-scan] module verdict={:?} rules={}",
                response.verdict,
                response.rule_ids.join(",")
            );
            use crate::modules::security::text::ScanVerdict;
            Some(ModuleScan {
                verdict: match response.verdict {
                    ScanVerdict::Clean => ModuleVerdict::Clean,
                    ScanVerdict::Suspicious => ModuleVerdict::Suspicious,
                    ScanVerdict::Blocked => ModuleVerdict::Blocked,
                },
                rule_ids: response.rule_ids,
            })
        }
        Err(error) => {
            log::debug!("[security][prompt-scan] module unavailable: {error}");
            None
        }
    }
}

/// Without the module feature there is no second opinion.
#[cfg(not(feature = "security-module"))]
pub async fn module_scan(_text: &str) -> Option<ModuleScan> {
    None
}

#[cfg(test)]
#[path = "module_scan_tests.rs"]
mod tests;
