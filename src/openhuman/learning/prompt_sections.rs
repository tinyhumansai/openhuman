//! Prompt sections that inject learned context into the agent's system prompt.
//!
//! These sections read from memory at prompt-build time and format observations,
//! patterns, and user profile data for the LLM context.

use crate::openhuman::agent::prompt::{PromptContext, PromptSection};
use crate::openhuman::memory::{Memory, MemoryCategory};
use anyhow::Result;
use std::sync::Arc;

/// Injects recent observations and patterns from the learning subsystem.
pub struct LearnedContextSection {
    memory: Arc<dyn Memory>,
}

impl LearnedContextSection {
    pub fn new(memory: Arc<dyn Memory>) -> Self {
        Self { memory }
    }

    /// Query memory synchronously using a blocking task (prompt building is sync).
    fn load_learned_context(&self) -> String {
        let memory = self.memory.clone();
        let rt = tokio::runtime::Handle::try_current();
        let (observations, patterns) = match rt {
            Ok(handle) => {
                let handle2 = handle.clone();
                let mem = memory.clone();
                let obs = std::thread::spawn(move || {
                    handle.block_on(async {
                        mem.list(
                            Some(&MemoryCategory::Custom("learning_observations".into())),
                            None,
                        )
                        .await
                        .unwrap_or_default()
                    })
                })
                .join()
                .unwrap_or_default();

                let mem2 = memory;
                let pats = std::thread::spawn(move || {
                    handle2.block_on(async {
                        mem2.list(
                            Some(&MemoryCategory::Custom("learning_patterns".into())),
                            None,
                        )
                        .await
                        .unwrap_or_default()
                    })
                })
                .join()
                .unwrap_or_default();

                (obs, pats)
            }
            Err(_) => {
                log::debug!("[learning] no tokio runtime for prompt section, skipping");
                return String::new();
            }
        };

        if observations.is_empty() && patterns.is_empty() {
            return String::new();
        }

        let mut out = String::from("## Learned Context\n\n");

        if !observations.is_empty() {
            out.push_str("### Recent Observations\n");
            // Show most recent 5 observations
            for entry in observations.iter().rev().take(5) {
                out.push_str("- ");
                out.push_str(entry.content.trim());
                out.push('\n');
            }
            out.push('\n');
        }

        if !patterns.is_empty() {
            out.push_str("### Recognized Patterns\n");
            for entry in patterns.iter().take(3) {
                out.push_str("- ");
                out.push_str(entry.content.trim());
                out.push('\n');
            }
            out.push('\n');
        }

        out
    }
}

impl PromptSection for LearnedContextSection {
    fn name(&self) -> &str {
        "learned_context"
    }

    fn build(&self, _ctx: &PromptContext<'_>) -> Result<String> {
        Ok(self.load_learned_context())
    }
}

/// Injects the learned user profile into the system prompt.
pub struct UserProfileSection {
    memory: Arc<dyn Memory>,
}

impl UserProfileSection {
    pub fn new(memory: Arc<dyn Memory>) -> Self {
        Self { memory }
    }

    fn load_user_profile(&self) -> String {
        let memory = self.memory.clone();
        let rt = tokio::runtime::Handle::try_current();
        let entries = match rt {
            Ok(handle) => {
                let mem = memory;
                std::thread::spawn(move || {
                    handle.block_on(async {
                        mem.list(
                            Some(&MemoryCategory::Custom("user_profile".into())),
                            None,
                        )
                        .await
                        .unwrap_or_default()
                    })
                })
                .join()
                .unwrap_or_default()
            }
            Err(_) => return String::new(),
        };

        if entries.is_empty() {
            return String::new();
        }

        let mut out = String::from("## User Profile (Learned)\n\n");
        for entry in entries.iter().take(20) {
            out.push_str("- ");
            out.push_str(entry.content.trim());
            out.push('\n');
        }
        out.push('\n');
        out
    }
}

impl PromptSection for UserProfileSection {
    fn name(&self) -> &str {
        "user_profile"
    }

    fn build(&self, _ctx: &PromptContext<'_>) -> Result<String> {
        Ok(self.load_user_profile())
    }
}
