use serde_json::Value;
use tinycomputer_bus::agent::TaskId;
use tinycomputer_bus::browser::{Action, Target};

/// A task paused before an irreversible action, waiting for host approval.
pub(super) struct Pending {
    pub(super) task: TaskId,
    pub(super) action: String,
    pub(super) target: String,
    pub(super) token: String,
}

impl Pending {
    pub(super) fn matches(&self, args: &Value) -> bool {
        args["token"].as_str() == Some(self.token.as_str())
    }
}

pub(super) fn needs_host_confirmation(action: &Action) -> bool {
    matches!(
        action,
        Action::Click { .. }
            | Action::DoubleClick { .. }
            | Action::Fill { .. }
            | Action::Type { .. }
            | Action::Press { .. }
            | Action::Select { .. }
            | Action::Check { .. }
    )
}

pub(super) fn approval_target(action: &Action) -> (Option<&str>, String) {
    let target = match action {
        Action::Click { target, .. }
        | Action::DoubleClick { target }
        | Action::Fill { target, .. }
        | Action::Select { target, .. }
        | Action::Check { target, .. } => Some(target),
        Action::Type { target, .. } | Action::Press { target, .. } => target.as_ref(),
        _ => None,
    };
    let preview = |raw: &str| {
        let cleaned = raw.chars().filter(|c| !c.is_control()).collect::<String>();
        let mut short = cleaned.chars().take(96).collect::<String>();
        if cleaned.chars().count() > 96 {
            short.push('…');
        }
        short
    };
    match target {
        Some(Target::Ref { value }) => (Some(value), format!(" @{value}")),
        Some(Target::Selector { value }) => (None, format!(" CSS selector {:?}", preview(value))),
        Some(Target::Locator { value }) => {
            let name = value
                .name
                .as_deref()
                .map(|name| format!(" named {:?}", preview(name)))
                .unwrap_or_default();
            (
                None,
                format!(
                    " {:?} locator {:?}{name} (match {}, exact={})",
                    value.by,
                    preview(&value.value),
                    value.index.saturating_add(1),
                    value.exact
                ),
            )
        }
        None => (None, String::new()),
    }
}
