//! Row conversion for the SQLite notification store.

use crate::storage::rusqlite;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};

use super::super::types::{IntegrationNotification, NotificationStatus};

pub(super) fn rows_to_notifications(
    mut rows: rusqlite::Rows<'_>,
) -> Result<Vec<IntegrationNotification>> {
    let mut out = Vec::new();
    while let Some(row) = rows
        .next()
        .context("[notifications::store] row iteration failed")?
    {
        out.push(row_to_notification(row)?);
    }
    Ok(out)
}

fn row_to_notification(row: &rusqlite::Row<'_>) -> Result<IntegrationNotification> {
    let raw_payload_str: String = row.get(5)?;
    let raw_payload: serde_json::Value = serde_json::from_str(&raw_payload_str)
        .unwrap_or(serde_json::Value::String(raw_payload_str));

    let status_str: String = row.get(9)?;
    let status = match status_str.as_str() {
        "read" => NotificationStatus::Read,
        "acted" => NotificationStatus::Acted,
        "dismissed" => NotificationStatus::Dismissed,
        _ => NotificationStatus::Unread,
    };

    let received_at_str: String = row.get(10)?;
    let received_at: DateTime<Utc> = received_at_str.parse().unwrap_or_else(|e| {
        tracing::warn!(
            raw = %received_at_str,
            error = %e,
            "[notifications::store] invalid received_at, using now"
        );
        Utc::now()
    });

    let scored_at_str: Option<String> = row.get(11)?;
    let scored_at: Option<DateTime<Utc>> = scored_at_str.and_then(|s| match s.parse() {
        Ok(t) => Some(t),
        Err(e) => {
            tracing::warn!(
                raw = %s,
                error = %e,
                "[notifications::store] invalid scored_at, treating as unscored"
            );
            None
        }
    });

    Ok(IntegrationNotification {
        id: row.get(0)?,
        provider: row.get(1)?,
        account_id: row.get(2)?,
        title: row.get(3)?,
        body: row.get(4)?,
        raw_payload,
        importance_score: row.get(6)?,
        triage_action: row.get(7)?,
        triage_reason: row.get(8)?,
        status,
        received_at,
        scored_at,
    })
}
