use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum JobType {
    #[default]
    Shell,
    Agent,
}

impl JobType {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Shell => "shell",
            Self::Agent => "agent",
        }
    }

    pub(crate) fn parse(raw: &str) -> Self {
        if raw.eq_ignore_ascii_case("agent") {
            Self::Agent
        } else {
            Self::Shell
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum SessionTarget {
    #[default]
    Isolated,
    Main,
}

impl SessionTarget {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Isolated => "isolated",
            Self::Main => "main",
        }
    }

    pub(crate) fn parse(raw: &str) -> Self {
        if raw.eq_ignore_ascii_case("main") {
            Self::Main
        } else {
            Self::Isolated
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActiveHours {
    pub start: String,
    pub end: String,
}

/// A cron-job schedule.
///
/// Serializes as an internally-tagged object (`{"kind": "cron", ...}`).
/// Deserializes from **either** that object form **or** a bare cron-expression
/// string like `"0 9 * * 1"` — the bare-string form is treated as
/// `Schedule::Cron { expr, tz: None, active_hours: None }`.
///
/// The bare-string shorthand exists because agents and some older frontend
/// callers pass `schedule: "0 9 * * 1"` directly instead of the structured
/// object.  Accepting it here prevents Sentry issue CORE-RUST-FY
/// ("invalid type: string, expected internally tagged enum Schedule").
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Schedule {
    Cron {
        expr: String,
        #[serde(default)]
        tz: Option<String>,
        #[serde(default)]
        active_hours: Option<ActiveHours>,
    },
    At {
        at: DateTime<Utc>,
    },
    Every {
        every_ms: u64,
    },
}

impl<'de> Deserialize<'de> for Schedule {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::{self, MapAccess, Visitor};
        use std::fmt;

        struct ScheduleVisitor;

        impl<'de> Visitor<'de> for ScheduleVisitor {
            type Value = Schedule;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(
                    f,
                    "a cron-schedule object ({{\"kind\":\"cron\",\"expr\":\"...\"}}) \
                     or a bare cron-expression string"
                )
            }

            /// Accept a bare string as `Schedule::Cron { expr, .. }`.
            /// This handles callers that send `schedule: "0 9 * * 1"` directly
            /// instead of the structured form.
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                tracing::debug!(
                    "[cron] Schedule::deserialize: got bare string '{}', \
                     coercing to Cron variant",
                    value
                );
                Ok(Schedule::Cron {
                    expr: value.to_owned(),
                    tz: None,
                    active_hours: None,
                })
            }

            fn visit_string<E: de::Error>(self, value: String) -> Result<Self::Value, E> {
                tracing::debug!(
                    "[cron] Schedule::deserialize: got bare string '{}', \
                     coercing to Cron variant",
                    value
                );
                Ok(Schedule::Cron {
                    expr: value,
                    tz: None,
                    active_hours: None,
                })
            }

            /// Accept the standard internally-tagged object form.
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
                // Delegate to the serde-derived tagged-enum logic by
                // deserializing from a collected map value.
                #[derive(Deserialize)]
                #[serde(tag = "kind", rename_all = "lowercase")]
                enum ScheduleTagged {
                    Cron {
                        expr: String,
                        #[serde(default)]
                        tz: Option<String>,
                        #[serde(default)]
                        active_hours: Option<ActiveHours>,
                    },
                    At {
                        at: DateTime<Utc>,
                    },
                    Every {
                        every_ms: u64,
                    },
                }

                let tagged =
                    ScheduleTagged::deserialize(de::value::MapAccessDeserializer::new(map))?;
                Ok(match tagged {
                    ScheduleTagged::Cron {
                        expr,
                        tz,
                        active_hours,
                    } => Schedule::Cron {
                        expr,
                        tz,
                        active_hours,
                    },
                    ScheduleTagged::At { at } => Schedule::At { at },
                    ScheduleTagged::Every { every_ms } => Schedule::Every { every_ms },
                })
            }
        }

        deserializer.deserialize_any(ScheduleVisitor)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeliveryConfig {
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
    #[serde(default = "default_true")]
    pub best_effort: bool,
}

impl Default for DeliveryConfig {
    fn default() -> Self {
        Self {
            mode: "none".to_string(),
            channel: None,
            to: None,
            best_effort: true,
        }
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJob {
    pub id: String,
    pub expression: String,
    pub schedule: Schedule,
    pub command: String,
    pub prompt: Option<String>,
    pub name: Option<String>,
    pub job_type: JobType,
    pub session_target: SessionTarget,
    pub model: Option<String>,
    /// Optional built-in agent definition ID (e.g. `"welcome"`,
    /// `"morning_briefing"`). When set, [`crate::openhuman::cron::scheduler`]
    /// resolves the agent definition from the registry and runs with the
    /// definition's prompt, tool allowlist, iteration cap, and model hint
    /// instead of the generic `Agent::from_config` path.
    pub agent_id: Option<String>,
    pub enabled: bool,
    pub delivery: DeliveryConfig,
    pub delete_after_run: bool,
    pub created_at: DateTime<Utc>,
    pub next_run: DateTime<Utc>,
    pub last_run: Option<DateTime<Utc>>,
    pub last_status: Option<String>,
    pub last_output: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronRun {
    pub id: i64,
    pub job_id: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub status: String,
    pub output: Option<String>,
    pub duration_ms: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CronJobPatch {
    pub schedule: Option<Schedule>,
    pub command: Option<String>,
    pub prompt: Option<String>,
    pub name: Option<String>,
    pub enabled: Option<bool>,
    pub delivery: Option<DeliveryConfig>,
    pub model: Option<String>,
    pub session_target: Option<SessionTarget>,
    pub delete_after_run: Option<bool>,
    pub agent_id: Option<Option<String>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use serde_json::json;

    // ── JobType ────────────────────────────────────────────────────

    #[test]
    fn job_type_parse_and_as_str_roundtrip() {
        assert_eq!(JobType::parse("shell").as_str(), "shell");
        assert_eq!(JobType::parse("agent").as_str(), "agent");
        // Case-insensitive
        assert_eq!(JobType::parse("AGENT"), JobType::Agent);
        assert_eq!(JobType::parse("Agent"), JobType::Agent);
        // Anything unknown falls back to Shell (the default) — guards
        // against unexpected legacy DB rows silently turning into Agent.
        assert_eq!(JobType::parse(""), JobType::Shell);
        assert_eq!(JobType::parse("garbage"), JobType::Shell);
    }

    #[test]
    fn job_type_default_is_shell() {
        assert_eq!(JobType::default(), JobType::Shell);
    }

    #[test]
    fn job_type_serializes_lowercase() {
        assert_eq!(serde_json::to_string(&JobType::Shell).unwrap(), "\"shell\"");
        assert_eq!(serde_json::to_string(&JobType::Agent).unwrap(), "\"agent\"");
    }

    // ── SessionTarget ──────────────────────────────────────────────

    #[test]
    fn session_target_parse_and_as_str_roundtrip() {
        assert_eq!(SessionTarget::parse("isolated").as_str(), "isolated");
        assert_eq!(SessionTarget::parse("main").as_str(), "main");
        // Case-insensitive + unknown falls back to Isolated (the default).
        assert_eq!(SessionTarget::parse("MAIN"), SessionTarget::Main);
        assert_eq!(SessionTarget::parse(""), SessionTarget::Isolated);
        assert_eq!(SessionTarget::parse("unknown"), SessionTarget::Isolated);
    }

    #[test]
    fn session_target_default_is_isolated() {
        assert_eq!(SessionTarget::default(), SessionTarget::Isolated);
    }

    #[test]
    fn session_target_serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&SessionTarget::Isolated).unwrap(),
            "\"isolated\""
        );
        assert_eq!(
            serde_json::to_string(&SessionTarget::Main).unwrap(),
            "\"main\""
        );
    }

    // ── Schedule ───────────────────────────────────────────────────

    #[test]
    fn schedule_cron_variant_roundtrips_with_optional_tz() {
        let s = Schedule::Cron {
            expr: "0 9 * * *".into(),
            tz: Some("America/Los_Angeles".into()),
            active_hours: None,
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["kind"], "cron");
        assert_eq!(v["expr"], "0 9 * * *");
        assert_eq!(v["tz"], "America/Los_Angeles");
        let back: Schedule = serde_json::from_value(v).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn schedule_cron_variant_accepts_missing_tz() {
        let raw = json!({ "kind": "cron", "expr": "*/5 * * * *" });
        let s: Schedule = serde_json::from_value(raw).unwrap();
        assert_eq!(
            s,
            Schedule::Cron {
                expr: "*/5 * * * *".into(),
                tz: None,
                active_hours: None,
            }
        );
    }

    #[test]
    fn schedule_cron_variant_roundtrips_with_active_hours() {
        let s = Schedule::Cron {
            expr: "*/15 * * * *".into(),
            tz: Some("UTC".into()),
            active_hours: Some(ActiveHours {
                start: "09:00".into(),
                end: "17:30".into(),
            }),
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["active_hours"]["start"], "09:00");
        assert_eq!(v["active_hours"]["end"], "17:30");
        let back: Schedule = serde_json::from_value(v).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn schedule_at_variant_roundtrips_with_utc_timestamp() {
        let at = Utc.with_ymd_and_hms(2027, 1, 15, 12, 0, 0).unwrap();
        let s = Schedule::At { at };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["kind"], "at");
        let back: Schedule = serde_json::from_value(v).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn schedule_every_variant_roundtrips() {
        let s = Schedule::Every { every_ms: 60_000 };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["kind"], "every");
        assert_eq!(v["every_ms"], 60_000);
        let back: Schedule = serde_json::from_value(v).unwrap();
        assert_eq!(back, s);
    }

    // ── Schedule bare-string deserialization (CORE-RUST-FY fix) ──────
    // Callers (agents, older frontend) sometimes pass a bare cron
    // expression string like `"0 9 * * 1"` instead of the structured
    // `{"kind":"cron","expr":"0 9 * * 1"}` form.  Both must parse.

    #[test]
    fn schedule_deserializes_bare_cron_string() {
        let s: Schedule = serde_json::from_value(json!("0 9 * * 1")).unwrap();
        assert_eq!(
            s,
            Schedule::Cron {
                expr: "0 9 * * 1".into(),
                tz: None,
                active_hours: None,
            }
        );
    }

    #[test]
    fn schedule_deserializes_bare_5_field_cron_string() {
        let s: Schedule = serde_json::from_str("\"*/5 * * * *\"").unwrap();
        assert_eq!(
            s,
            Schedule::Cron {
                expr: "*/5 * * * *".into(),
                tz: None,
                active_hours: None,
            }
        );
    }

    #[test]
    fn cron_job_patch_accepts_bare_schedule_string() {
        // This is the exact payload shape that triggered CORE-RUST-FY:
        // {"schedule": "0 9 * * 1"}
        let raw = json!({ "schedule": "0 9 * * 1" });
        let patch: CronJobPatch = serde_json::from_value(raw).unwrap();
        assert_eq!(
            patch.schedule,
            Some(Schedule::Cron {
                expr: "0 9 * * 1".into(),
                tz: None,
                active_hours: None,
            })
        );
    }

    #[test]
    fn cron_job_patch_still_accepts_structured_schedule_object() {
        let raw = json!({ "schedule": { "kind": "cron", "expr": "0 9 * * 1" } });
        let patch: CronJobPatch = serde_json::from_value(raw).unwrap();
        assert_eq!(
            patch.schedule,
            Some(Schedule::Cron {
                expr: "0 9 * * 1".into(),
                tz: None,
                active_hours: None,
            })
        );
    }

    // ── DeliveryConfig ─────────────────────────────────────────────

    #[test]
    fn delivery_config_default_is_none_mode_best_effort() {
        let d = DeliveryConfig::default();
        assert_eq!(d.mode, "none");
        assert!(d.channel.is_none());
        assert!(d.to.is_none());
        assert!(d.best_effort, "default best_effort must be true");
    }

    #[test]
    fn delivery_config_parses_empty_object_with_defaults() {
        // A bare `{}` must deserialize with the `#[serde(default)]` / default
        // fn fallbacks — otherwise legacy rows without delivery fields would
        // fail to load.
        let d: DeliveryConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(d.mode, "");
        assert!(d.channel.is_none());
        assert!(d.to.is_none());
        assert!(d.best_effort, "best_effort must default to true");
    }

    #[test]
    fn delivery_config_preserves_best_effort_false_override() {
        let raw = json!({ "mode": "channel", "best_effort": false });
        let d: DeliveryConfig = serde_json::from_value(raw).unwrap();
        assert_eq!(d.mode, "channel");
        assert!(!d.best_effort);
    }

    // ── CronJobPatch ───────────────────────────────────────────────

    #[test]
    fn cron_job_patch_default_is_all_none() {
        let p = CronJobPatch::default();
        assert!(p.schedule.is_none());
        assert!(p.command.is_none());
        assert!(p.prompt.is_none());
        assert!(p.name.is_none());
        assert!(p.enabled.is_none());
        assert!(p.delivery.is_none());
        assert!(p.model.is_none());
        assert!(p.session_target.is_none());
        assert!(p.delete_after_run.is_none());
        assert!(p.agent_id.is_none());
    }

    #[test]
    fn cron_job_patch_agent_id_supports_explicit_none_clearing() {
        // Option<Option<String>> lets callers distinguish "no change"
        // (None) from "clear the agent_id" (Some(None)).
        let p = CronJobPatch {
            agent_id: Some(None),
            ..Default::default()
        };
        assert!(p.agent_id.is_some());
        assert!(p.agent_id.as_ref().unwrap().is_none());
    }
}
