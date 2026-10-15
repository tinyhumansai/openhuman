//! Fold the legacy security tables into the single `[security]` table.
//!
//! Runs on the parsed document before the typed decode, on every load, so the
//! result does not depend on whether the workspace crossed schema version 15:
//!
//! * `[security.autonomy]`, `[security.sandbox]` and `[security.privacy]` are
//!   the canonical spelling. They are overlaid key by key onto the legacy
//!   top-level `[autonomy]`, `[sandbox]` and `[privacy]` tables, and the
//!   `[security.*]` value wins. Consumers keep reading `Config::{autonomy,
//!   sandbox, privacy}`.
//! * A document with neither spelling is untouched, so the policy stays off
//!   (`autonomy.enabled` defaults to `false`).
//! * `autonomy.max_cost_per_day_cents` is deleted: nothing enforced it, and the
//!   policy module no longer advertises a cost budget (tinysecurity ADR 0006).
//!   Each removal is reported through [`Notice`] and logged.

use toml::Value;

/// The three tables that move under `[security]`.
const FOLDED: [&str; 3] = ["autonomy", "sandbox", "privacy"];

/// A user-visible record of something the rewrite changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Notice {
    /// A retired `max_cost_per_day_cents` key was dropped from this table.
    DroppedMaxCost { table: &'static str },
    /// `[security.<table>]` was overlaid onto the legacy `[<table>]`.
    Overlaid { table: &'static str },
}

pub(crate) fn rewrite_document(document: &mut Value) -> Vec<Notice> {
    let mut notices = Vec::new();
    let Some(root) = document.as_table_mut() else {
        return notices;
    };
    if let Some(Value::Table(autonomy)) = root.get_mut("autonomy") {
        if autonomy.remove("max_cost_per_day_cents").is_some() {
            notices.push(Notice::DroppedMaxCost { table: "autonomy" });
        }
    }
    let mut overlays = Vec::new();
    if let Some(Value::Table(security)) = root.get_mut("security") {
        for name in FOLDED {
            if let Some(Value::Table(mut table)) = security.remove(name) {
                if name == "autonomy" && table.remove("max_cost_per_day_cents").is_some() {
                    notices.push(Notice::DroppedMaxCost {
                        table: "security.autonomy",
                    });
                }
                overlays.push((name, table));
            }
        }
    }
    for (name, table) in overlays {
        let target = root
            .entry(name.to_string())
            .or_insert_with(|| Value::Table(Default::default()));
        if !target.is_table() {
            *target = Value::Table(Default::default());
        }
        if let Value::Table(target) = target {
            for (key, value) in table {
                target.insert(key, value);
            }
        }
        notices.push(Notice::Overlaid { table: name });
    }
    for notice in &notices {
        match notice {
            Notice::DroppedMaxCost { table } => log::warn!(
                "[migrations][security] removed retired {table}.max_cost_per_day_cents: \
                 no cost budget is enforced"
            ),
            Notice::Overlaid { table } => {
                log::debug!("[migrations][security] [security.{table}] overlaid onto [{table}]")
            }
        }
    }
    notices
}

#[cfg(test)]
#[path = "security_policy_tests.rs"]
mod tests;
