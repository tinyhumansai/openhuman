//! Tool: current_time — returns the current time in UTC and local time zones.
//!
//! Gives the orchestrator (and other agents) a way to ground reasoning that
//! depends on "now" — reminders, scheduling, relative date parsing — without
//! having to shell out to `date`. Read-only, no arguments beyond an optional
//! IANA timezone for a convenience conversion.

use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use chrono::{Local, SecondsFormat, Utc};
use chrono_tz::Tz;
use serde_json::json;

pub struct CurrentTimeTool;

impl CurrentTimeTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for CurrentTimeTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for CurrentTimeTool {
    fn name(&self) -> &str {
        "current_time"
    }

    fn description(&self) -> &str {
        "Get the current date and time in UTC and the machine's local timezone. \
         Optionally convert to a specific IANA timezone (e.g. 'America/Los_Angeles', \
         'Asia/Kolkata'). Use before scheduling reminders / cron jobs or when the \
         user refers to relative times like 'in 10 minutes', 'tomorrow', 'tonight'."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "timezone": {
                    "type": "string",
                    "description": "Optional IANA timezone name (e.g. 'Europe/London'). \
                                    If omitted, only UTC and machine-local are returned."
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        tracing::debug!(args = %args, "[current_time] execute start");
        let now_utc = Utc::now();
        let now_local = Local::now();

        let mut payload = json!({
            "utc": now_utc.to_rfc3339_opts(SecondsFormat::Secs, true),
            "local": now_local.to_rfc3339_opts(SecondsFormat::Secs, true),
            "local_timezone": now_local.format("%Z").to_string(),
            "unix_seconds": now_utc.timestamp(),
            "weekday": now_local.format("%A").to_string(),
        });

        if let Some(tz_name) = args.get("timezone").and_then(|v| v.as_str()) {
            let trimmed = tz_name.trim();
            tracing::debug!(
                tz_name = tz_name,
                trimmed = trimmed,
                now_utc = %now_utc,
                now_local = %now_local,
                "[current_time] normalized timezone input"
            );
            if !trimmed.is_empty() {
                match trimmed.parse::<Tz>() {
                    Ok(tz) => {
                        let converted = now_utc.with_timezone(&tz);
                        tracing::debug!(
                            trimmed = trimmed,
                            converted = %converted,
                            "[current_time] timezone conversion succeeded"
                        );
                        payload["requested_timezone"] = json!({
                            "name": trimmed,
                            "time": converted.to_rfc3339_opts(SecondsFormat::Secs, true),
                            "weekday": converted.format("%A").to_string(),
                        });
                    }
                    Err(error) => {
                        tracing::debug!(
                            trimmed = trimmed,
                            error = %error,
                            "[current_time] timezone conversion failed"
                        );
                        payload["requested_timezone_error"] = json!(format!(
                            "Unknown IANA timezone '{trimmed}' — use names like 'America/Los_Angeles'."
                        ));
                    }
                }
            }
        }

        tracing::debug!("[current_time] returning payload: {payload}");
        Ok(ToolResult::success(serde_json::to_string_pretty(&payload)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_and_permission() {
        let tool = CurrentTimeTool::new();
        assert_eq!(tool.name(), "current_time");
        assert_eq!(tool.permission_level(), PermissionLevel::ReadOnly);
    }

    #[test]
    fn schema_is_object() {
        let schema = CurrentTimeTool::new().parameters_schema();
        assert_eq!(schema["type"], "object");
    }

    #[tokio::test]
    async fn returns_utc_and_local() {
        let result = CurrentTimeTool::new().execute(json!({})).await.unwrap();
        assert!(!result.is_error);
        let payload: serde_json::Value = serde_json::from_str(&result.output()).unwrap();
        assert!(payload["utc"].is_string());
        assert!(payload["local"].is_string());
        assert!(payload["unix_seconds"].is_number());
    }

    #[tokio::test]
    async fn converts_requested_timezone() {
        let result = CurrentTimeTool::new()
            .execute(json!({ "timezone": "Asia/Kolkata" }))
            .await
            .unwrap();
        assert!(!result.is_error);
        let payload: serde_json::Value = serde_json::from_str(&result.output()).unwrap();
        assert!(payload["requested_timezone"].is_object());
        assert!(payload["requested_timezone"]["name"].is_string());
        assert!(
            payload["requested_timezone"]["name"]
                .as_str()
                .unwrap()
                .contains("Asia/Kolkata")
        );
    }

    #[tokio::test]
    async fn unknown_timezone_reports_error_field() {
        let result = CurrentTimeTool::new()
            .execute(json!({ "timezone": "Not/AReal_Zone" }))
            .await
            .unwrap();
        assert!(!result.is_error);
        let payload: serde_json::Value = serde_json::from_str(&result.output()).unwrap();
        assert!(payload["requested_timezone_error"].is_string());
    }
}
