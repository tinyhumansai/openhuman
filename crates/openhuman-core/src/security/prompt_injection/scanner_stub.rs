//! A scripted [`PromptScanner`] for the tests of what the host does with the
//! scanner's answers (blocking, holding, windowing, deadlines) without loading
//! the security module. It is not the module: what the module scores is pinned by
//! the module's own corpus (`tinysecurity-redact`).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;

use super::scanner::{DefinitionScan, DefinitionVerdict, PromptScan, PromptScanner, Tier};

/// How the stub answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// A few unmistakable injection phrases are `Block`, the rest `Allow`.
    Phrases,
    /// Every call fails, as a module that cannot answer does.
    Unavailable,
    /// Every call never returns, as a hung module does.
    Hang,
}

/// Phrases the stub treats as an injection, lower case.
const PHRASES: &[&str] = &[
    "ignore all previous instructions",
    "ignore previous instructions",
    "reveal the system prompt",
    "reveal your system prompt",
    "jailbreak",
];

/// The scripted scanner.
#[derive(Debug)]
pub(crate) struct StubScanner {
    mode: Mode,
    prompt_calls: AtomicUsize,
    seen: std::sync::Mutex<Vec<usize>>,
}

impl StubScanner {
    fn with(mode: Mode) -> Arc<Self> {
        Arc::new(Self {
            mode,
            prompt_calls: AtomicUsize::new(0),
            seen: std::sync::Mutex::new(Vec::new()),
        })
    }

    /// Blocks the stub's injection phrases and allows everything else.
    pub(crate) fn phrases() -> Arc<Self> {
        Self::with(Mode::Phrases)
    }

    /// Fails every call.
    pub(crate) fn unavailable() -> Arc<Self> {
        Self::with(Mode::Unavailable)
    }

    /// Never answers.
    pub(crate) fn hanging() -> Arc<Self> {
        Self::with(Mode::Hang)
    }

    /// How many prompt scans were asked.
    pub(crate) fn prompt_calls(&self) -> usize {
        self.prompt_calls.load(Ordering::SeqCst)
    }

    /// The byte length of each text asked, in order.
    pub(crate) fn lengths(&self) -> Vec<usize> {
        self.seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn hit(text: &str) -> bool {
        let lowered = text.to_lowercase();
        PHRASES.iter().any(|phrase| lowered.contains(phrase))
    }
}

#[async_trait]
impl PromptScanner for StubScanner {
    async fn scan_prompt(&self, text: &str) -> Result<PromptScan, String> {
        self.prompt_calls.fetch_add(1, Ordering::SeqCst);
        self.seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(text.len());
        match self.mode {
            Mode::Unavailable => Err("the stub scanner is unavailable".into()),
            Mode::Hang => std::future::pending().await,
            Mode::Phrases if Self::hit(text) => Ok(PromptScan {
                tier: Tier::Block,
                score_milli: 1000,
                rule_ids: vec!["override.ignore_previous".into()],
            }),
            Mode::Phrases => Ok(PromptScan::clean()),
        }
    }

    async fn scan_tool_definition(
        &self,
        name: &str,
        definition: &serde_json::Value,
    ) -> Result<DefinitionScan, String> {
        match self.mode {
            Mode::Unavailable => Err("the stub scanner is unavailable".into()),
            Mode::Hang => std::future::pending().await,
            Mode::Phrases if Self::hit(name) || Self::hit(&definition.to_string()) => {
                Ok(DefinitionScan {
                    verdict: DefinitionVerdict::Blocked,
                    rule_ids: vec!["override.instructions".into()],
                })
            }
            Mode::Phrases => Ok(DefinitionScan {
                verdict: DefinitionVerdict::Clean,
                rule_ids: Vec::new(),
            }),
        }
    }
}
