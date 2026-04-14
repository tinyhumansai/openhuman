//! Task classification and routing policy.
//!
//! Maps `hint:*` model strings to task categories and produces deterministic
//! routing decisions based on task category, local model availability, and
//! caller-supplied routing hints.

/// Task complexity tier for model selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskCategory {
    /// Reactions, short classifications, simple formatting. Local-first.
    Lightweight,
    /// Summarization, limited tool orchestration. Local-preferred.
    Medium,
    /// Deep reasoning, long-context planning, complex generation. Remote only.
    Heavy,
}

impl TaskCategory {
    /// Human-readable label for telemetry.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lightweight => "lightweight",
            Self::Medium => "medium",
            Self::Heavy => "heavy",
        }
    }
}

/// Latency priority for a routing call.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LatencyBudget {
    /// Prefer the lowest-latency path (local).
    Low,
    #[default]
    Normal,
}

/// Cost sensitivity for a routing call.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CostSensitivity {
    #[default]
    Normal,
    /// Minimize token cost — prefer local.
    High,
}

/// Per-call routing hints that influence the policy decision.
///
/// All fields default to the permissive/normal setting so callers only need
/// to set the fields that matter.
#[derive(Debug, Clone, Default)]
pub struct RoutingHints {
    /// When `true` the request must never leave the local runtime. No fallback
    /// to remote is permitted even when local fails or returns low quality.
    pub privacy_required: bool,
    /// Bias toward the lowest-latency path (local model).
    pub latency_budget: LatencyBudget,
    /// Bias toward the lowest-cost path (local model).
    pub cost_sensitivity: CostSensitivity,
}

/// Routing target produced by the policy decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutingTarget {
    /// Use the local model with the given model ID.
    Local { model: String },
    /// Use the remote backend with the given model string (may be a `hint:*`).
    Remote { model: String },
}

impl RoutingTarget {
    /// Human-readable label for telemetry.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Local { .. } => "local",
            Self::Remote { .. } => "remote",
        }
    }

    /// The resolved model string passed to the chosen provider.
    pub fn model(&self) -> &str {
        match self {
            Self::Local { model } | Self::Remote { model } => model,
        }
    }
}

/// Classify a model string (possibly `hint:*`) into a task category.
///
/// Rules:
/// - `hint:reaction`, `hint:classify`, `hint:format`, `hint:sentiment`,
///   `hint:lightweight` → [`TaskCategory::Lightweight`]
/// - `hint:summarize`, `hint:medium`, `hint:tool_lite` → [`TaskCategory::Medium`]
/// - All other `hint:*` values and exact model names → [`TaskCategory::Heavy`]
pub fn classify(model: &str) -> TaskCategory {
    match model.strip_prefix("hint:") {
        Some("reaction" | "classify" | "format" | "sentiment" | "lightweight") => {
            TaskCategory::Lightweight
        }
        Some("summarize" | "medium" | "tool_lite") => TaskCategory::Medium,
        _ => TaskCategory::Heavy,
    }
}

/// Decide where to route a task.
///
/// Returns `(primary, fallback)` where `fallback` is `Some` only when the
/// primary target is local and fallback to remote is permitted. A `None`
/// fallback means the caller must not retry on another backend.
///
/// # Privacy override
/// When `hints.privacy_required` is `true` the request is always routed
/// locally and no fallback is produced, regardless of category or health.
///
/// # Heavy tasks
/// Heavy tasks always use remote unless `privacy_required` forces local.
///
/// # Local preference
/// Lightweight and medium tasks use local when `local_available` is true.
pub fn decide(
    category: TaskCategory,
    local_model: &str,
    remote_model: &str,
    local_available: bool,
    hints: &RoutingHints,
) -> (RoutingTarget, Option<RoutingTarget>) {
    // Privacy override: always local, never fall back.
    if hints.privacy_required {
        return (
            RoutingTarget::Local {
                model: local_model.to_string(),
            },
            None,
        );
    }

    // Heavy tasks always go to remote.
    if category == TaskCategory::Heavy {
        return (
            RoutingTarget::Remote {
                model: remote_model.to_string(),
            },
            None,
        );
    }

    // Lightweight / Medium: prefer local when available.
    let use_local =
        local_available && matches!(category, TaskCategory::Lightweight | TaskCategory::Medium);

    if use_local {
        (
            RoutingTarget::Local {
                model: local_model.to_string(),
            },
            Some(RoutingTarget::Remote {
                model: remote_model.to_string(),
            }),
        )
    } else {
        (
            RoutingTarget::Remote {
                model: remote_model.to_string(),
            },
            None,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_hints() -> RoutingHints {
        RoutingHints::default()
    }

    // ── classify ──────────────────────────────────────────────────────────────

    #[test]
    fn lightweight_hints_classify_correctly() {
        for hint in &[
            "hint:reaction",
            "hint:classify",
            "hint:format",
            "hint:sentiment",
            "hint:lightweight",
        ] {
            assert_eq!(
                classify(hint),
                TaskCategory::Lightweight,
                "{hint} should be Lightweight"
            );
        }
    }

    #[test]
    fn medium_hints_classify_correctly() {
        for hint in &["hint:summarize", "hint:medium", "hint:tool_lite"] {
            assert_eq!(
                classify(hint),
                TaskCategory::Medium,
                "{hint} should be Medium"
            );
        }
    }

    #[test]
    fn heavy_hints_classify_correctly() {
        for hint in &[
            "hint:reasoning",
            "hint:agentic",
            "hint:coding",
            "hint:heavy",
            "hint:fast",
            "hint:unknown_future_hint",
        ] {
            assert_eq!(
                classify(hint),
                TaskCategory::Heavy,
                "{hint} should be Heavy"
            );
        }
    }

    #[test]
    fn exact_model_name_is_heavy() {
        assert_eq!(classify("gemma3:4b-it-qat"), TaskCategory::Heavy);
        assert_eq!(classify("neocortex-mk1"), TaskCategory::Heavy);
        assert_eq!(classify(""), TaskCategory::Heavy);
    }

    // ── decide: basic routing ─────────────────────────────────────────────────

    #[test]
    fn lightweight_local_healthy_routes_local_with_fallback() {
        let (primary, fallback) = decide(
            TaskCategory::Lightweight,
            "local-model",
            "remote-model",
            true,
            &default_hints(),
        );
        assert_eq!(
            primary,
            RoutingTarget::Local {
                model: "local-model".into()
            }
        );
        assert_eq!(
            fallback,
            Some(RoutingTarget::Remote {
                model: "remote-model".into()
            })
        );
    }

    #[test]
    fn lightweight_local_unavailable_routes_remote_no_fallback() {
        let (primary, fallback) = decide(
            TaskCategory::Lightweight,
            "local-model",
            "remote-model",
            false,
            &default_hints(),
        );
        assert_eq!(
            primary,
            RoutingTarget::Remote {
                model: "remote-model".into()
            }
        );
        assert!(fallback.is_none());
    }

    #[test]
    fn medium_local_healthy_routes_local() {
        let (primary, fallback) = decide(
            TaskCategory::Medium,
            "local-model",
            "remote-model",
            true,
            &default_hints(),
        );
        assert_eq!(
            primary,
            RoutingTarget::Local {
                model: "local-model".into()
            }
        );
        assert!(fallback.is_some());
    }

    #[test]
    fn heavy_always_routes_remote_regardless_of_health() {
        for local_healthy in [true, false] {
            let (primary, fallback) = decide(
                TaskCategory::Heavy,
                "local-model",
                "remote-model",
                local_healthy,
                &default_hints(),
            );
            assert_eq!(
                primary,
                RoutingTarget::Remote {
                    model: "remote-model".into()
                },
                "heavy tasks must always go remote (local_healthy={local_healthy})"
            );
            assert!(fallback.is_none());
        }
    }

    // ── decide: privacy override ──────────────────────────────────────────────

    #[test]
    fn privacy_required_forces_local_no_fallback() {
        let hints = RoutingHints {
            privacy_required: true,
            ..Default::default()
        };
        // Even for heavy tasks and when local is unhealthy
        for category in [
            TaskCategory::Lightweight,
            TaskCategory::Medium,
            TaskCategory::Heavy,
        ] {
            for local_available in [true, false] {
                let (primary, fallback) = decide(
                    category,
                    "local-model",
                    "remote-model",
                    local_available,
                    &hints,
                );
                assert_eq!(
                    primary,
                    RoutingTarget::Local { model: "local-model".into() },
                    "privacy_required must always route local (category={:?}, local_available={local_available})",
                    category
                );
                assert!(
                    fallback.is_none(),
                    "privacy_required must never produce a remote fallback"
                );
            }
        }
    }

    // ── decide: latency / cost signals ───────────────────────────────────────

    #[test]
    fn low_latency_budget_routes_local_when_available() {
        let hints = RoutingHints {
            latency_budget: LatencyBudget::Low,
            ..Default::default()
        };
        let (primary, _) = decide(
            TaskCategory::Lightweight,
            "local-model",
            "remote-model",
            true,
            &hints,
        );
        assert!(matches!(primary, RoutingTarget::Local { .. }));
    }

    #[test]
    fn high_cost_sensitivity_routes_local_when_available() {
        let hints = RoutingHints {
            cost_sensitivity: CostSensitivity::High,
            ..Default::default()
        };
        let (primary, _) = decide(
            TaskCategory::Lightweight,
            "local-model",
            "remote-model",
            true,
            &hints,
        );
        assert!(matches!(primary, RoutingTarget::Local { .. }));
    }

    #[test]
    fn low_latency_does_not_override_heavy_to_local() {
        let hints = RoutingHints {
            latency_budget: LatencyBudget::Low,
            ..Default::default()
        };
        let (primary, _) = decide(
            TaskCategory::Heavy,
            "local-model",
            "remote-model",
            true,
            &hints,
        );
        // Heavy tasks are always remote even with low latency budget
        assert!(matches!(primary, RoutingTarget::Remote { .. }));
    }

    // ── regressions ──────────────────────────────────────────────────────────

    #[test]
    fn regression_reasoning_always_remote() {
        let category = classify("hint:reasoning");
        assert_eq!(category, TaskCategory::Heavy);
        let (primary, _) = decide(
            category,
            "local-model",
            "hint:reasoning",
            true,
            &default_hints(),
        );
        assert_eq!(
            primary,
            RoutingTarget::Remote {
                model: "hint:reasoning".into()
            }
        );
    }

    #[test]
    fn regression_agentic_always_remote() {
        let category = classify("hint:agentic");
        let (primary, _) = decide(
            category,
            "local-model",
            "hint:agentic",
            true,
            &default_hints(),
        );
        assert!(matches!(primary, RoutingTarget::Remote { .. }));
    }

    #[test]
    fn routing_target_helpers() {
        let local = RoutingTarget::Local { model: "m".into() };
        assert_eq!(local.label(), "local");
        assert_eq!(local.model(), "m");

        let remote = RoutingTarget::Remote { model: "r".into() };
        assert_eq!(remote.label(), "remote");
        assert_eq!(remote.model(), "r");
    }

    #[test]
    fn task_category_as_str() {
        assert_eq!(TaskCategory::Lightweight.as_str(), "lightweight");
        assert_eq!(TaskCategory::Medium.as_str(), "medium");
        assert_eq!(TaskCategory::Heavy.as_str(), "heavy");
    }
}
