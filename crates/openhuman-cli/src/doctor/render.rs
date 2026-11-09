use serde_json::Value;

use super::args::DoctorArgs;

#[cfg(test)]
pub(super) fn render_table(report: &Value) -> String {
    render(report, None)
}

pub(super) fn render_table_for_args(report: &Value, options: &DoctorArgs) -> String {
    let minimum = options.lint.then_some(options.severity_min.as_str());
    render(report, minimum)
}

fn render(report: &Value, minimum: Option<&str>) -> String {
    let findings = report
        .get("findings")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut groups: Vec<(String, Vec<&Value>)> = Vec::new();

    for finding in findings {
        let severity = finding
            .get("severity")
            .and_then(Value::as_str)
            .unwrap_or("info");
        if minimum.is_some_and(|minimum| !meets_threshold(severity, minimum)) {
            continue;
        }
        let id = finding
            .get("check_id")
            .and_then(Value::as_str)
            .unwrap_or("unknown/check");
        let category = id.split_once('/').map_or("other", |(area, _)| area);
        let index = groups
            .iter()
            .position(|(existing, _)| existing == category)
            .unwrap_or_else(|| {
                groups.push((category.to_owned(), Vec::new()));
                groups.len() - 1
            });
        groups[index].1.push(finding);
    }

    let mut output = String::new();
    render_profile_header(report, &mut output);
    for (category, findings) in groups {
        output.push_str(&category);
        output.push_str(":\n");
        for finding in findings {
            let severity = finding
                .get("severity")
                .and_then(Value::as_str)
                .unwrap_or("info");
            let id = finding
                .get("check_id")
                .and_then(Value::as_str)
                .unwrap_or("unknown/check");
            let message = finding
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("No message was provided.");
            let fix_hint = finding
                .get("fix_hint")
                .and_then(Value::as_str)
                .unwrap_or("Review this check and rerun doctor.");
            output.push_str(&format!(
                "  [{severity}] {id} {message}\n  fix: {fix_hint}\n"
            ));
        }
    }

    let empty_summary = Value::Null;
    let summary = report.get("summary").unwrap_or(&empty_summary);
    output.push_str(&format!(
        "Summary: pass {}, info {}, warn {}, error {}, skipped {}\n",
        count(summary, "pass"),
        count(summary, "info"),
        count(summary, "warn"),
        count(summary, "error"),
        count(summary, "skipped"),
    ));
    output
}

fn render_profile_header(report: &Value, output: &mut String) {
    let profile = report.get("profile").and_then(Value::as_str);
    let source = report
        .get("config_resolution_source")
        .and_then(Value::as_str);
    let layout = report.get("profile_layout_version").and_then(Value::as_u64);
    if profile.is_none() && source.is_none() && layout.is_none() {
        return;
    }

    output.push_str("Profile: ");
    output.push_str(profile.unwrap_or("(none)"));
    if let Some(layout) = layout {
        output.push_str(&format!(" | layout v{layout}"));
    }
    if let Some(source) = source {
        output.push_str(&format!(" | config source: {source}"));
    }
    output.push('\n');
}

fn count(summary: &Value, severity: &str) -> u64 {
    summary.get(severity).and_then(Value::as_u64).unwrap_or(0)
}

fn meets_threshold(severity: &str, minimum: &str) -> bool {
    severity_rank(severity) >= severity_rank(minimum) && severity_rank(severity) > 0
}

fn severity_rank(severity: &str) -> u8 {
    match severity {
        "info" => 1,
        "warn" => 2,
        "error" => 3,
        _ => 0,
    }
}
