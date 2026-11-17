//! Bounded, inspectable tool and child-run projections.
use super::theme::safe_text;
use crate::event_view::WebEventView;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Running,
    Success,
    Error,
    Waiting,
    Cancelled,
    Unknown,
}
impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Success => "done",
            Self::Error => "failed",
            Self::Waiting => "needs input",
            Self::Cancelled => "cancelled",
            Self::Unknown => "stale",
        }
    }
    pub fn marker(self) -> &'static str {
        match self {
            Self::Running => "·",
            Self::Success => "✓",
            Self::Error => "✗",
            Self::Waiting => "!",
            Self::Cancelled => "–",
            Self::Unknown => "?",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activity {
    pub id: String,
    pub label: String,
    pub status: Status,
    pub args: String,
    pub output: String,
    pub history: String,
    pub elapsed_ms: Option<u64>,
    pub child: bool,
    pub tools: u64,
}

impl Activity {
    pub fn from_event(id: String, event: &WebEventView, child: bool) -> Self {
        Self {
            id,
            label: safe_text(
                event
                    .tool_display_label
                    .as_deref()
                    .or(event.tool_name.as_deref())
                    .unwrap_or(if child { "Subagent" } else { "Tool" }),
            ),
            status: Status::Running,
            args: String::new(),
            output: String::new(),
            history: String::new(),
            elapsed_ms: None,
            child,
            tools: 0,
        }
    }
    pub fn update(&mut self, event: &WebEventView) {
        if let Some(args) = &event.args {
            self.args = bounded(&args.to_string(), 4096);
        }
        if let Some(elapsed) = event.elapsed_ms.filter(|_| {
            !self.child
                || matches!(
                    event.event.as_str(),
                    "subagent_completed" | "subagent_failed"
                )
        }) {
            self.elapsed_ms = Some(elapsed);
        }
        if self.child {
            if let Some(detail) = &event.subagent {
                if let Some(name) = &detail.display_name {
                    self.label = safe_text(name);
                }
                if let Some(elapsed) = detail.elapsed_ms {
                    self.elapsed_ms = Some(elapsed);
                }
                if let Some(output) = &detail.output {
                    self.output = bounded(output, 65536);
                }
            }
            match event.event.as_str() {
                "subagent_completed" => self.status = Status::Success,
                "subagent_failed" => self.status = Status::Error,
                "subagent_awaiting_user" => self.status = Status::Waiting,
                "subagent_spawned" => self.status = Status::Running,
                "subagent_tool_call" => {
                    self.tools += 1;
                    append_bounded(
                        &mut self.history,
                        &format!(
                            "\nTool: {}\n{}",
                            event.tool_name.as_deref().unwrap_or("tool"),
                            self.args
                        ),
                        65536,
                    );
                }
                "subagent_tool_result" => {
                    append_bounded(
                        &mut self.history,
                        &format!(
                            "\nResult: {}\n{}",
                            if event.success == Some(false) {
                                "failed"
                            } else {
                                "done"
                            },
                            event.output.as_deref().unwrap_or("No output")
                        ),
                        65536,
                    );
                }
                "subagent_text_delta" => {
                    if let Some(delta) = &event.delta {
                        append_bounded(&mut self.output, delta, 65536);
                    }
                }
                _ => {}
            }
            if matches!(
                event.event.as_str(),
                "subagent_failed" | "subagent_awaiting_user"
            ) {
                append_bounded(
                    &mut self.history,
                    &format!(
                        "\n{}: {}",
                        self.status.label(),
                        event.message.as_deref().unwrap_or("No details supplied")
                    ),
                    65536,
                );
            }
        } else if event.event == "tool_result" {
            self.status = if event.success == Some(false) {
                Status::Error
            } else {
                Status::Success
            };
            if let Some(output) = &event.output {
                self.output = bounded(output, 65536);
            } else if self.output.is_empty() {
                self.output = "No output".into();
            }
            if let Some(failure) = &event.failure {
                self.output = bounded(&format!("{}\n{failure}", self.output), 65536);
            }
        }
    }
    pub fn summary(&self) -> String {
        let time = self
            .elapsed_ms
            .map(|ms| format!(" · {ms} ms"))
            .unwrap_or_default();
        let count = if self.child {
            format!(" · {} tools", self.tools)
        } else {
            String::new()
        };
        format!(
            "{} {} · {}{count}{time}",
            self.status.marker(),
            self.label,
            self.status.label()
        )
    }
    pub fn details(&self) -> String {
        let mut result = String::new();
        if !self.args.is_empty() {
            result.push_str("Arguments\n");
            result.push_str(&self.args);
            result.push('\n');
        }
        result.push_str(if self.child {
            "Child output (read-only)\n"
        } else {
            "Output\n"
        });
        result.push_str(if self.output.is_empty() {
            "No output available yet"
        } else {
            &self.output
        });
        if !self.history.is_empty() {
            result.push_str("\nTool activity\n");
            result.push_str(&self.history);
        }
        result
    }
}

pub fn bounded(value: &str, limit: usize) -> String {
    let safe = safe_text(value);
    if safe.len() <= limit {
        return safe;
    }
    let mut end = limit;
    while !safe.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n[preview truncated]", &safe[..end])
}
fn append_bounded(value: &mut String, delta: &str, limit: usize) {
    if value.len() >= limit {
        return;
    }
    *value = bounded(&format!("{value}{delta}"), limit);
}
