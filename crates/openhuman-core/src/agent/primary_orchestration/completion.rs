//! Machine-checkable completion contracts and observations for Phase 7.
//!
//! Every turn has a machine-checkable completion contract:
//! - `chat`: non-empty final assistant text;
//! - `sourced_web`: final text containing the requested bounded result and sources;
//! - `image_retrieval`: at least one validated image result/source rendered to the client,
//!   not merely a search-results page URL;
//! - `artifact_generation`: an existing artifact path/URL plus final explanation;
//! - `repository_mutation`: recorded change plus requested verification status;
//! - `scheduling`: persisted schedule identifier and state;
//! - `clarification`: explicit yielded question;
//! - `approval`: explicit yielded approval request.
//!
//! Successful informational tools (e.g. search, fetch, read, inspect) are NOT completion.
//! The state machine normally performs one final model call, or uses a deterministic renderer
//! when the tool returns a complete typed result and model summarization is unnecessary or unavailable.

use serde::{Deserialize, Serialize};

use super::intent::{IntentCompletion, RequestIntent};

/// Typed machine-checkable completion contract for a primary turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionContract {
    Chat,
    SourcedWeb { min_sources: usize },
    ImageRetrieval { require_direct_media: bool },
    ArtifactGeneration { require_explanation: bool },
    RepositoryMutation { require_verification: bool },
    Scheduling,
    Clarification,
    Approval,
}

/// Verification status for repository changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationStatus {
    Verified,
    Failed,
    Unverified,
}

/// A validated image result with its source origin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidatedImage {
    pub url_or_path: String,
    pub source_origin: String,
    pub mime_type: Option<String>,
}

/// A generated artifact record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneratedArtifact {
    pub path_or_url: String,
    pub description: Option<String>,
}

/// A recorded change to a repository with verification state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryChangeRecord {
    pub file_paths: Vec<String>,
    pub verification: VerificationStatus,
    pub summary: Option<String>,
}

/// A persisted scheduling entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduleRecord {
    pub schedule_id: String,
    pub state: String,
}

/// Observed turn evidence collected across model passes and tool executions.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletionObservation {
    pub final_assistant_text: Option<String>,
    pub informational_tools_executed: Vec<String>,
    pub web_sources: Vec<String>,
    pub validated_images: Vec<ValidatedImage>,
    pub generated_artifacts: Vec<GeneratedArtifact>,
    pub repository_changes: Vec<RepositoryChangeRecord>,
    pub scheduling_records: Vec<ScheduleRecord>,
    pub yielded_question: Option<String>,
    pub yielded_approval: Option<String>,
}

/// The evaluation outcome of matching turn observations against a completion contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionStatus {
    Complete,
    Incomplete {
        reason: String,
        needs_final_model_call: bool,
    },
    FalseCompletion {
        reason: String,
    },
}

impl CompletionStatus {
    pub fn is_complete(&self) -> bool {
        matches!(self, Self::Complete)
    }

    pub fn is_incomplete(&self) -> bool {
        matches!(self, Self::Incomplete { .. })
    }

    pub fn is_false_completion(&self) -> bool {
        matches!(self, Self::FalseCompletion { .. })
    }
}

/// Helper to detect whether a URL points to a search results page rather than an image.
pub fn is_search_page_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.contains("google.com/search")
        || lower.contains("bing.com/search")
        || lower.contains("bing.com/images/search")
        || lower.contains("duckduckgo.com/?q=")
        || lower.contains("duckduckgo.com/html")
        || lower.contains("search?")
        || lower.contains("/search?")
}

/// Map a resolved RequestIntent into its authoritative CompletionContract.
pub fn contract_from_intent(intent: &RequestIntent) -> CompletionContract {
    match intent.completion {
        IntentCompletion::FinalText => CompletionContract::Chat,
        IntentCompletion::SourcedAnswer => CompletionContract::SourcedWeb { min_sources: 1 },
        IntentCompletion::ImageResult => CompletionContract::ImageRetrieval {
            require_direct_media: true,
        },
        IntentCompletion::Artifact => CompletionContract::ArtifactGeneration {
            require_explanation: true,
        },
        IntentCompletion::MemoryResult => CompletionContract::Chat,
        IntentCompletion::VerifiedChange => CompletionContract::RepositoryMutation {
            require_verification: true,
        },
        IntentCompletion::ScheduleState => CompletionContract::Scheduling,
        IntentCompletion::DelegatedResult => CompletionContract::Chat,
    }
}

/// Evaluate whether observed execution satisfies the machine-checkable contract.
pub fn evaluate_completion(
    contract: &CompletionContract,
    obs: &CompletionObservation,
) -> CompletionStatus {
    match contract {
        CompletionContract::Chat => {
            if let Some(text) = &obs.final_assistant_text {
                if !text.trim().is_empty() {
                    CompletionStatus::Complete
                } else {
                    CompletionStatus::Incomplete {
                        reason: "Assistant text is empty".into(),
                        needs_final_model_call: true,
                    }
                }
            } else if !obs.informational_tools_executed.is_empty() {
                CompletionStatus::Incomplete {
                    reason: "Informational tools executed without final assistant text".into(),
                    needs_final_model_call: true,
                }
            } else {
                CompletionStatus::Incomplete {
                    reason: "No assistant text produced".into(),
                    needs_final_model_call: true,
                }
            }
        }

        CompletionContract::SourcedWeb { min_sources } => {
            if let Some(text) = &obs.final_assistant_text {
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    return CompletionStatus::Incomplete {
                        reason: "Final assistant text is empty".into(),
                        needs_final_model_call: true,
                    };
                }
                let count = obs.web_sources.len();
                if count < *min_sources {
                    let has_embedded_source = trimmed.contains("http://")
                        || trimmed.contains("https://")
                        || trimmed.contains("Source:")
                        || trimmed.contains("Sources:");
                    if !has_embedded_source {
                        return CompletionStatus::FalseCompletion {
                            reason: format!(
                                "Web answer lacks required sources (found {}, required {})",
                                count, min_sources
                            ),
                        };
                    }
                }
                CompletionStatus::Complete
            } else if !obs.informational_tools_executed.is_empty() {
                CompletionStatus::Incomplete {
                    reason: "Informational search/fetch completed; requires final model response or deterministic rendering".into(),
                    needs_final_model_call: true,
                }
            } else {
                CompletionStatus::Incomplete {
                    reason: "No search results or response text produced".into(),
                    needs_final_model_call: true,
                }
            }
        }

        CompletionContract::ImageRetrieval {
            require_direct_media,
        } => {
            if obs.validated_images.is_empty() {
                if !obs.informational_tools_executed.is_empty() {
                    CompletionStatus::FalseCompletion {
                        reason:
                            "Navigation or search executed without validated image result rendered"
                                .into(),
                    }
                } else {
                    CompletionStatus::Incomplete {
                        reason: "No image results produced".into(),
                        needs_final_model_call: true,
                    }
                }
            } else {
                let mut valid_count = 0;
                for img in &obs.validated_images {
                    if is_search_page_url(&img.url_or_path) {
                        return CompletionStatus::FalseCompletion {
                            reason: format!(
                                "Search results page URL '{}' is not a validated image result",
                                img.url_or_path
                            ),
                        };
                    }
                    if *require_direct_media {
                        let has_media_hint = img
                            .mime_type
                            .as_deref()
                            .is_some_and(|m| m.starts_with("image/"))
                            || img.url_or_path.ends_with(".png")
                            || img.url_or_path.ends_with(".jpg")
                            || img.url_or_path.ends_with(".jpeg")
                            || img.url_or_path.ends_with(".webp")
                            || img.url_or_path.ends_with(".gif")
                            || img.url_or_path.ends_with(".svg")
                            || img.url_or_path.starts_with("data:image/")
                            || img.url_or_path.starts_with("file://");
                        if has_media_hint || !img.source_origin.is_empty() {
                            valid_count += 1;
                        }
                    } else {
                        valid_count += 1;
                    }
                }

                if valid_count > 0 {
                    CompletionStatus::Complete
                } else {
                    CompletionStatus::FalseCompletion {
                        reason: "Images failed direct media validation".into(),
                    }
                }
            }
        }

        CompletionContract::ArtifactGeneration {
            require_explanation,
        } => {
            if obs.generated_artifacts.is_empty() {
                CompletionStatus::Incomplete {
                    reason: "No artifact generated".into(),
                    needs_final_model_call: true,
                }
            } else if *require_explanation {
                if let Some(text) = &obs.final_assistant_text {
                    if !text.trim().is_empty() {
                        CompletionStatus::Complete
                    } else {
                        CompletionStatus::Incomplete {
                            reason: "Artifact generated but final explanation required".into(),
                            needs_final_model_call: true,
                        }
                    }
                } else {
                    CompletionStatus::Incomplete {
                        reason: "Artifact generated but final explanation required".into(),
                        needs_final_model_call: true,
                    }
                }
            } else {
                CompletionStatus::Complete
            }
        }

        CompletionContract::RepositoryMutation {
            require_verification,
        } => {
            if obs.repository_changes.is_empty() {
                if !obs.informational_tools_executed.is_empty() {
                    CompletionStatus::Incomplete {
                        reason: "Informational inspection only; repository mutation not recorded"
                            .into(),
                        needs_final_model_call: true,
                    }
                } else {
                    CompletionStatus::Incomplete {
                        reason: "No repository change recorded".into(),
                        needs_final_model_call: true,
                    }
                }
            } else if *require_verification {
                let any_verified = obs
                    .repository_changes
                    .iter()
                    .any(|c| c.verification == VerificationStatus::Verified);
                let any_failed = obs
                    .repository_changes
                    .iter()
                    .any(|c| c.verification == VerificationStatus::Failed);

                if any_failed {
                    CompletionStatus::FalseCompletion {
                        reason: "Repository mutation verification failed".into(),
                    }
                } else if any_verified {
                    CompletionStatus::Complete
                } else {
                    CompletionStatus::FalseCompletion {
                        reason: "Repository mutation unverified; verification required".into(),
                    }
                }
            } else {
                CompletionStatus::Complete
            }
        }

        CompletionContract::Scheduling => {
            if obs.scheduling_records.is_empty() {
                CompletionStatus::Incomplete {
                    reason: "No schedule record produced".into(),
                    needs_final_model_call: true,
                }
            } else {
                let valid = obs
                    .scheduling_records
                    .iter()
                    .any(|s| !s.schedule_id.trim().is_empty() && !s.state.trim().is_empty());
                if valid {
                    CompletionStatus::Complete
                } else {
                    CompletionStatus::FalseCompletion {
                        reason: "Schedule record has empty identifier or state".into(),
                    }
                }
            }
        }

        CompletionContract::Clarification => {
            if let Some(q) = &obs.yielded_question {
                if !q.trim().is_empty() {
                    CompletionStatus::Complete
                } else {
                    CompletionStatus::Incomplete {
                        reason: "Clarification question is empty".into(),
                        needs_final_model_call: true,
                    }
                }
            } else {
                CompletionStatus::Incomplete {
                    reason: "Clarification question required".into(),
                    needs_final_model_call: true,
                }
            }
        }

        CompletionContract::Approval => {
            if let Some(a) = &obs.yielded_approval {
                if !a.trim().is_empty() {
                    CompletionStatus::Complete
                } else {
                    CompletionStatus::Incomplete {
                        reason: "Approval request is empty".into(),
                        needs_final_model_call: true,
                    }
                }
            } else {
                CompletionStatus::Incomplete {
                    reason: "Approval request required".into(),
                    needs_final_model_call: true,
                }
            }
        }
    }
}

/// Render a deterministic complete response when summarization is unnecessary or unavailable.
pub fn render_deterministic_completion(
    contract: &CompletionContract,
    obs: &CompletionObservation,
) -> Option<String> {
    match contract {
        CompletionContract::ImageRetrieval { .. } => {
            let img = obs.validated_images.first()?;
            Some(format!(
                "Found image from {}:\n![Image]({})",
                img.source_origin, img.url_or_path
            ))
        }
        CompletionContract::Scheduling => {
            let s = obs.scheduling_records.first()?;
            Some(format!(
                "Schedule action confirmed: ID {} is {}",
                s.schedule_id, s.state
            ))
        }
        CompletionContract::RepositoryMutation { .. } => {
            let r = obs.repository_changes.first()?;
            let files = r.file_paths.join(", ");
            Some(format!(
                "Successfully applied verified changes to: {} (status: {:?})",
                files, r.verification
            ))
        }
        CompletionContract::SourcedWeb { .. } => {
            if obs.web_sources.is_empty() {
                return None;
            }
            let mut out = String::from("Sourced web results:\n");
            for src in &obs.web_sources {
                out.push_str(&format!("- {}\n", src));
            }
            Some(out)
        }
        _ => None,
    }
}
