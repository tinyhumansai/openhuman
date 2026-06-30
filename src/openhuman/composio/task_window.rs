//! Recency-window narrowing for Composio **task-fetch** actions.
//!
//! Background: the `morning_briefing` agent fetches the user's open tasks via
//! `composio_execute` against whichever task manager they connected. Left
//! unbounded those calls return the *entire* backlog; the brief only wants
//! what was created/changed in the last 24h. The cron runner installs a
//! [`crate::openhuman::agent::harness::current_task_recency_window`] for the
//! brief turn, and this module applies it inside the `composio_execute`
//! handler.
//!
//! Two layers, both gated on the task-local window being present AND the slug
//! appearing in [`spec_for`]:
//!
//! 1. **Best-effort server-side narrowing** ([`apply_window_args`]): inject an
//!    ordering / `*_since` argument the provider understands, when the caller
//!    didn't supply one. Caller-supplied values always win. This only reduces
//!    payload size / improves ordering — correctness never depends on it.
//! 2. **Authoritative client-side post-filter** ([`filter_response`]): drop
//!    rows whose timestamp predates `now - window`. This is the enforcement.
//!    Mirrors the proven `sync_depth_days` floor in the native sync providers
//!    (e.g. `memory_sync::composio::providers::linear::provider`).
//!
//! Scope is intentionally narrow: only slugs with a *verified* response shape
//! are listed. An unknown slug degrades to "no filtering" — never a crash and never a
//! silently-emptied result. An unrecognized envelope or an unparseable
//! timestamp is treated as "keep", so a wrong field-map degrades to no-op.
//!
//! `markdownFormatted` note: in backend mode the response often carries a
//! server-rendered markdown string that the tool surface prefers over `data`.
//! When the post-filter removes rows it clears `markdown_formatted` so the
//! agent reads the *filtered* JSON instead of the stale full-backlog markdown.

use chrono::{DateTime, Utc};
use serde_json::Value;

use super::types::ComposioExecuteResponse;

/// How a provider encodes a task timestamp, so the post-filter can compare it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TsFormat {
    /// RFC3339 / ISO-8601 string, e.g. `2026-06-21T10:00:00.000Z`.
    Iso8601,
    /// Unix epoch **milliseconds**, as a JSON string or number (ClickUp).
    EpochMillis,
}

/// Per-slug knowledge needed to narrow + filter a task-fetch response.
struct TaskWindowSpec {
    /// Candidate key-paths to the items array inside `resp.data`. The first
    /// path that resolves to a JSON array wins (providers differ in whether
    /// they nest under `data`, `results`, `nodes`, …). An empty path means
    /// `resp.data` is itself the array.
    items_paths: &'static [&'static [&'static str]],
    /// Timestamp fields on each item, checked in order. A row is **kept** if
    /// *any* listed field parses to a time `>= floor`. A row whose fields are
    /// all missing/unparseable is also kept (we never drop on ambiguity).
    ts_fields: &'static [(&'static str, TsFormat)],
    /// Optional static ordering arg to inject (key, value) when absent.
    order_arg: Option<(&'static str, &'static str)>,
}

/// Verified task-fetch slugs. Keep this list to slugs whose response shape is
/// confirmed (repo-proven or docs-verified). See module docs.
fn spec_for(slug: &str) -> Option<TaskWindowSpec> {
    match slug {
        // Linear — repo-proven request (`orderBy:"updatedAt"`) and response
        // shape. Linear returns GraphQL connections (`{issues:{nodes:[...]}}`)
        // that Composio may re-wrap under `data` / `data.data`; the items and
        // timestamp paths mirror `providers::linear::sync::{extract_issues,
        // extract_issue_updated}` so we catch every nesting (else we'd hit the
        // no-array pass-through and leave the backlog unfiltered).
        "LINEAR_LIST_LINEAR_ISSUES" | "LINEAR_SEARCH_ISSUES" => Some(TaskWindowSpec {
            items_paths: &[
                &["data", "issues", "nodes"],
                &["data", "data", "nodes"],
                &["data", "nodes"],
                &["nodes"],
                &["issues", "nodes"],
                &["data", "results"],
                &["results"],
                &["data", "issues"],
                &["issues"],
            ],
            ts_fields: &[
                ("updatedAt", TsFormat::Iso8601),
                ("data.updatedAt", TsFormat::Iso8601),
                ("updated_at", TsFormat::Iso8601),
                ("data.updated_at", TsFormat::Iso8601),
            ],
            order_arg: Some(("orderBy", "updatedAt")),
        }),
        // ClickUp — repo-proven request (`order_by:"updated"`) and response
        // shape, mirroring `providers::clickup::sync::{extract_tasks,
        // extract_task_updated}` (envelope `data.tasks` / `tasks` /
        // `data.data.tasks`; timestamp `date_updated` epoch-ms, possibly
        // wrapped or camelCased by Composio).
        "CLICKUP_GET_FILTERED_TEAM_TASKS" | "CLICKUP_GET_TASKS" => Some(TaskWindowSpec {
            items_paths: &[
                &["data", "tasks"],
                &["tasks"],
                &["data", "data", "tasks"],
                &["data", "results"],
                &["results"],
            ],
            ts_fields: &[
                ("date_updated", TsFormat::EpochMillis),
                ("data.date_updated", TsFormat::EpochMillis),
                ("dateUpdated", TsFormat::EpochMillis),
                ("data.dateUpdated", TsFormat::EpochMillis),
            ],
            order_arg: Some(("order_by", "updated")),
        }),
        // Notion — docs-verified: every page carries `last_edited_time` and
        // `created_time`; query results live under `results` (Composio may
        // re-wrap under `data` / `data.data`).
        "NOTION_QUERY_DATABASE_WITH_FILTER" => Some(TaskWindowSpec {
            items_paths: &[
                &["results"],
                &["data", "results"],
                &["data", "data", "results"],
            ],
            ts_fields: &[
                ("last_edited_time", TsFormat::Iso8601),
                ("created_time", TsFormat::Iso8601),
                ("data.last_edited_time", TsFormat::Iso8601),
                ("data.created_time", TsFormat::Iso8601),
            ],
            order_arg: None,
        }),
        // Asana — docs-verified server-side `modified_since` (injected in
        // apply_window_args). Items live under `data` (Composio may re-wrap as
        // `data.data`); each row carries `modified_at` / `created_at`, possibly
        // `data`-wrapped.
        // CONFIRM-AT-RUNTIME: item timestamp field names via composio_list_tools.
        "ASANA_GET_MULTIPLE_TASKS" => Some(TaskWindowSpec {
            items_paths: &[&["data", "data"], &["data"]],
            ts_fields: &[
                ("modified_at", TsFormat::Iso8601),
                ("created_at", TsFormat::Iso8601),
                ("data.modified_at", TsFormat::Iso8601),
                ("data.created_at", TsFormat::Iso8601),
            ],
            order_arg: None,
        }),
        // Todoist — `TODOIST_GET_ALL_TASKS` (returns incomplete tasks; the
        // brief's catalog slug, fixed from the non-existent
        // `TODOIST_GET_ACTIVE_TASKS`). No server-side narrowing: GET tasks does
        // NOT accept a `filter` query (that lives on the separate
        // `TODOIST_FILTER_TASKS` / `/tasks/filter` endpoint), so enforcement is
        // pure post-filter. Todoist's v1 task object timestamps are `added_at`
        // (creation) and `updated_at` (modification) — keying on both gives
        // created-or-modified semantics. `created_at` is kept as a harmless
        // defensive fallback (extra fields only ever keep, never drop).
        // CONFIRM-AT-RUNTIME: response envelope via composio_list_tools (Todoist
        // is not a native sync provider, so there's no repo extractor to mirror).
        "TODOIST_GET_ALL_TASKS" => Some(TaskWindowSpec {
            items_paths: &[
                &["tasks"],
                &["data", "tasks"],
                &["results"],
                &["data", "results"],
                &["data"],
                &[],
            ],
            ts_fields: &[
                ("added_at", TsFormat::Iso8601),
                ("updated_at", TsFormat::Iso8601),
                ("created_at", TsFormat::Iso8601),
                ("data.added_at", TsFormat::Iso8601),
                ("data.updated_at", TsFormat::Iso8601),
                ("data.created_at", TsFormat::Iso8601),
            ],
            order_arg: None,
        }),
        _ => None,
    }
}

/// Inject best-effort server-side narrowing args for a task-fetch slug, when
/// the task-recency window is active. Caller-supplied keys are never
/// overwritten. Unknown slugs pass through untouched.
pub(crate) fn apply_window_args(
    slug: &str,
    arguments: Option<Value>,
    since: DateTime<Utc>,
) -> Option<Value> {
    let Some(spec) = spec_for(slug) else {
        return arguments;
    };

    let mut value = arguments.unwrap_or_else(|| Value::Object(Default::default()));
    let Some(map) = value.as_object_mut() else {
        // Non-object payload (unexpected) — leave it for the backend to reject.
        return Some(value);
    };

    if let Some((key, val)) = spec.order_arg {
        map.entry(key.to_string())
            .or_insert_with(|| Value::String(val.to_string()));
    }

    // Asana takes a dynamic `modified_since` rather than a static order arg.
    if slug == "ASANA_GET_MULTIPLE_TASKS" {
        map.entry("modified_since".to_string())
            .or_insert_with(|| Value::String(since.to_rfc3339()));
    }

    // Todoist has no server-side narrowing here: `TODOIST_GET_ALL_TASKS`
    // (GET /tasks) does not accept a `filter` query — that belongs to the
    // separate `TODOIST_FILTER_TASKS` (`/tasks/filter`) endpoint. Recency is
    // enforced entirely by the post-filter on `added_at`/`updated_at`.

    tracing::debug!(
        target: "composio",
        slug,
        since = %since.to_rfc3339(),
        "[composio][task-window] applied server-side narrowing args"
    );
    Some(value)
}

/// Drop task rows older than `since` from a successful response.
///
/// No-op when: the response failed, the slug isn't recognized, the items
/// array can't be located, or nothing was actually removed. When rows *are*
/// removed, `markdown_formatted` is cleared so the agent consumes the filtered
/// `data` rather than stale server-rendered markdown.
pub(crate) fn filter_response(
    slug: &str,
    mut resp: ComposioExecuteResponse,
    since: DateTime<Utc>,
) -> ComposioExecuteResponse {
    if !resp.successful {
        return resp;
    }
    let Some(spec) = spec_for(slug) else {
        return resp;
    };

    let Some(path) = spec
        .items_paths
        .iter()
        .find(|p| array_at(&resp.data, p).is_some())
        .copied()
    else {
        tracing::debug!(
            target: "composio",
            slug,
            "[composio][task-window] no items array located; pass-through"
        );
        return resp;
    };

    // Safe: `find` above guaranteed this path resolves to an array.
    let items = array_at(&resp.data, path).expect("path resolved above");
    let total = items.len();
    let kept: Vec<Value> = items
        .iter()
        .filter(|item| keep_item(item, spec.ts_fields, since))
        .cloned()
        .collect();
    let removed = total - kept.len();

    if removed == 0 {
        return resp;
    }

    if let Some(slot) = array_at_mut(&mut resp.data, path) {
        *slot = kept;
    }
    // Stale server-rendered markdown reflects the full backlog — force the
    // tool surface to fall back to the filtered JSON envelope.
    resp.markdown_formatted = None;

    tracing::debug!(
        target: "composio",
        slug,
        kept = total - removed,
        total,
        since = %since.to_rfc3339(),
        "[composio][task-window] post-filtered task results to recency window"
    );
    resp
}

/// Keep a row if any of its timestamp fields parses to `>= floor`. A row with
/// no parseable timestamp field is kept (never dropped on ambiguity).
///
/// Field names may be dotted (`data.updatedAt`) to reach Composio-wrapped rows
/// — mirrors `providers::pick_str`, which the native sync extractors use for
/// exactly these `data.`-nested envelopes.
fn keep_item(item: &Value, ts_fields: &[(&str, TsFormat)], floor: DateTime<Utc>) -> bool {
    let mut saw_timestamp = false;
    for (field, fmt) in ts_fields {
        if let Some(raw) = field_at(item, field) {
            if let Some(ts) = parse_ts(raw, *fmt) {
                saw_timestamp = true;
                if ts >= floor {
                    return true;
                }
            }
        }
    }
    // No usable timestamp at all → conservatively keep.
    !saw_timestamp
}

/// Resolve a possibly-dotted field path (`updatedAt`, `data.updatedAt`) within
/// a single item object. Each segment is an object key.
fn field_at<'a>(item: &'a Value, dotted: &str) -> Option<&'a Value> {
    let mut cur = item;
    for seg in dotted.split('.') {
        cur = cur.get(seg)?;
    }
    Some(cur)
}

/// Parse a JSON timestamp value in the given format. Returns `None` on any
/// shape we don't recognize so the caller treats it as ambiguous (keep).
fn parse_ts(raw: &Value, fmt: TsFormat) -> Option<DateTime<Utc>> {
    match fmt {
        TsFormat::Iso8601 => raw
            .as_str()
            .and_then(|s| DateTime::parse_from_rfc3339(s.trim()).ok())
            .map(|dt| dt.with_timezone(&Utc)),
        TsFormat::EpochMillis => {
            let ms = match raw {
                Value::String(s) => s.trim().parse::<i64>().ok(),
                Value::Number(n) => n.as_i64(),
                _ => None,
            }?;
            DateTime::from_timestamp_millis(ms)
        }
    }
}

/// Resolve `path` (a slice of object keys) to an array within `root`.
/// An empty path means `root` itself must be the array.
fn array_at<'a>(root: &'a Value, path: &[&str]) -> Option<&'a Vec<Value>> {
    let mut cur = root;
    for key in path {
        cur = cur.get(*key)?;
    }
    cur.as_array()
}

/// Mutable counterpart to [`array_at`].
fn array_at_mut<'a>(root: &'a mut Value, path: &[&str]) -> Option<&'a mut Vec<Value>> {
    let mut cur = root;
    for key in path {
        cur = cur.get_mut(*key)?;
    }
    cur.as_array_mut()
}

#[cfg(test)]
#[path = "task_window_tests.rs"]
mod tests;
