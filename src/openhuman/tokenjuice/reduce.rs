//! The main reduction pipeline: `reduce_execution` and helpers.
//!
//! Port of `src/core/reduce.ts` and the `normalizeExecutionInput` helper
//! from `src/core/command.ts`.

use std::collections::HashMap;

use crate::openhuman::tokenjuice::{
    classify::classify_execution,
    text::{
        clamp_text, clamp_text_middle, count_text_chars, dedupe_adjacent, head_tail,
        normalize_lines, pluralize, strip_ansi, trim_empty_edges,
    },
    types::{
        ClassificationResult, CompactResult, CompiledRule, CounterSource, ReduceOptions,
        ReductionStats, ToolExecutionInput,
    },
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Output shorter than this many chars is returned verbatim (passthrough) even
/// when a rule would compact it.
const TINY_OUTPUT_MAX_CHARS: usize = 240;

// ---------------------------------------------------------------------------
// Command normalisation (from command.ts)
// ---------------------------------------------------------------------------

/// Simple shell tokenizer (mirrors `tokenizeCommand` in TS).
pub fn tokenize_command(command: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaping = false;

    for ch in command.trim().chars() {
        if escaping {
            current.push(ch);
            escaping = false;
            continue;
        }
        if ch == '\\' {
            escaping = true;
            continue;
        }
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            } else {
                current.push(ch);
            }
            continue;
        }
        if ch == '\'' || ch == '"' {
            quote = Some(ch);
            continue;
        }
        if ch.is_whitespace() {
            if !current.is_empty() {
                tokens.push(current.clone());
                current.clear();
            }
            continue;
        }
        current.push(ch);
    }
    if escaping {
        current.push('\\');
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// Fill in `argv` from `command` if `argv` is absent.
pub fn normalize_execution_input(input: ToolExecutionInput) -> ToolExecutionInput {
    if input.argv.as_ref().map(|v| !v.is_empty()).unwrap_or(false) {
        return input;
    }
    let command = match &input.command {
        Some(c) if !c.is_empty() => c.clone(),
        _ => return input,
    };
    let argv = tokenize_command(&command);
    if argv.is_empty() {
        return input;
    }
    ToolExecutionInput {
        argv: Some(argv),
        ..input
    }
}

/// True when the command is a well-known file-content inspection tool.
pub fn is_file_content_inspection_command(input: &ToolExecutionInput) -> bool {
    static FILE_TOOLS: &[&str] = &[
        "cat", "sed", "head", "tail", "nl", "bat", "batcat", "jq", "yq",
    ];
    let argv = input.argv.as_deref().unwrap_or(&[]);
    if argv.is_empty() {
        return false;
    }
    let argv0 = std::path::Path::new(&argv[0])
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    FILE_TOOLS.contains(&argv0.as_str())
}

// ---------------------------------------------------------------------------
// Git-status post-processor
// ---------------------------------------------------------------------------

fn rewrite_git_status_line(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Some(String::new());
    }
    if trimmed.starts_with("On branch ") {
        return None;
    }
    // "and have N and M different commits each"
    if regex_match(r"^and have \d+ and \d+ different commits each", trimmed) {
        return None;
    }
    if regex_match(
        r"^(?:no changes added to commit|nothing added to commit but untracked files present)",
        trimmed,
    ) {
        return None;
    }
    if regex_match(r#"^\(use "git .+"\)$"#, trimmed)
        || regex_match(r#"^use "git .+" to .+"#, trimmed)
    {
        return None;
    }

    if trimmed == "Changes not staged for commit:" {
        return Some("Changes not staged:".to_owned());
    }
    if trimmed == "Changes to be committed:" {
        return Some("Staged changes:".to_owned());
    }
    if trimmed == "Untracked files:" {
        return Some("Untracked files:".to_owned());
    }

    if regex_match(r"^\s*modified:\s+", line) {
        let path = regex_replace(r"^\s*modified:\s+", line, "")
            .trim()
            .to_owned();
        return Some(format!("M: {}", path));
    }
    if regex_match(r"^\s*new file:\s+", line) {
        let path = regex_replace(r"^\s*new file:\s+", line, "")
            .trim()
            .to_owned();
        return Some(format!("A: {}", path));
    }
    if regex_match(r"^\s*deleted:\s+", line) {
        let path = regex_replace(r"^\s*deleted:\s+", line, "")
            .trim()
            .to_owned();
        return Some(format!("D: {}", path));
    }
    if regex_match(r"^\s*renamed:\s+", line) {
        let path = regex_replace(r"^\s*renamed:\s+", line, "")
            .trim()
            .to_owned();
        return Some(format!("R: {}", path));
    }
    if regex_match(r"^\?\?\s+", trimmed) {
        let path = regex_replace(r"^\?\?\s+", trimmed, "").trim().to_owned();
        return Some(format!("?? {}", path));
    }

    // Porcelain format: two status chars + space + path
    if let Some(caps) = regex_captures(r"^([ MADRCU?!]{2})\s+(.+)$", line) {
        let status_raw = caps[0].trim().replace('?', "??");
        let path = caps[1].trim();
        let code = if status_raw.is_empty() {
            "M"
        } else if status_raw.starts_with("??") {
            "??"
        } else {
            &status_raw[..1]
        };
        return Some(format!("{}: {}", code, path));
    }

    Some(trimmed.to_owned())
}

fn rewrite_git_status_lines(lines: &[String]) -> Vec<String> {
    let mut section: Option<&str> = None;

    let rewritten: Vec<Option<String>> = lines
        .iter()
        .map(|line| {
            let trimmed = line.trim();
            if trimmed == "Changes not staged for commit:" {
                section = Some("unstaged");
            } else if trimmed == "Changes to be committed:" {
                section = Some("staged");
            } else if trimmed == "Untracked files:" {
                section = Some("untracked");
            }

            // In untracked section, indented non-action lines become "?? "
            if section == Some("untracked")
                && regex_match(r"^\s{2,}\S", line)
                && !regex_match(r"^\s*(?:modified:|new file:|deleted:|renamed:)", line)
            {
                return Some(format!("?? {}", trimmed));
            }

            rewrite_git_status_line(line)
        })
        .collect();

    // Collapse consecutive empty lines
    let mut collapsed: Vec<String> = Vec::new();
    for line in rewritten.into_iter().flatten() {
        if line.is_empty() && collapsed.last().map(String::is_empty).unwrap_or(false) {
            continue;
        }
        collapsed.push(line);
    }
    collapsed
}

// ---------------------------------------------------------------------------
// GH output formatter
// ---------------------------------------------------------------------------

fn compact_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn format_gh_table_line(line: &str) -> String {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    // Split on 2+ spaces or tabs
    let columns: Vec<String> = regex::Regex::new(r"\s{2,}|\t+")
        .unwrap()
        .split(trimmed)
        .map(compact_whitespace)
        .filter(|s| !s.is_empty())
        .collect();

    if columns.len() >= 2 && regex_match(r"^\d+$", &columns[0]) {
        let number = &columns[0];
        let title = &columns[1];
        let state = if columns.len() >= 4 {
            columns.last()
        } else {
            None
        };
        let context = if columns.len() >= 3 {
            let end = if state.is_some() {
                columns.len() - 1
            } else {
                columns.len()
            };
            let slice = &columns[2..end];
            if slice.is_empty() {
                None
            } else {
                Some(slice.join(" "))
            }
        } else {
            None
        };
        let mut parts = vec![format!("#{}", number), title.clone()];
        if let Some(s) = state {
            parts.push(format!("[{}]", s));
        }
        if let Some(c) = context {
            parts.push(format!("({})", c));
        }
        return parts.join(" ");
    }
    compact_whitespace(trimmed)
}

fn rewrite_gh_lines(lines: &[String], input: &ToolExecutionInput) -> Vec<String> {
    let non_empty: Vec<&String> = lines.iter().filter(|l| !l.trim().is_empty()).collect();
    if non_empty.is_empty() {
        return Vec::new();
    }

    // Try to parse as JSON objects
    let parsed: Vec<Option<serde_json::Value>> = non_empty
        .iter()
        .map(|line| {
            let t = line.trim();
            if t.starts_with('{') && t.ends_with('}') {
                serde_json::from_str(t).ok()
            } else {
                None
            }
        })
        .collect();

    if parsed.iter().all(|p| p.is_some()) {
        let formatted: Vec<String> = parsed
            .into_iter()
            .filter_map(|v| format_gh_json_record(v?))
            .collect();
        if !formatted.is_empty() {
            return formatted;
        }
    }

    // Fall back to table formatting if argv[0] == "gh"
    let argv = input.argv.as_deref().unwrap_or(&[]);
    if argv.first().map(String::as_str) == Some("gh") {
        return lines.iter().map(|l| format_gh_table_line(l)).collect();
    }

    lines.to_vec()
}

fn format_gh_json_record(record: serde_json::Value) -> Option<String> {
    let obj = record.as_object()?;

    let title = obj
        .get("title")
        .and_then(|v| v.as_str())
        .or_else(|| obj.get("displayTitle").and_then(|v| v.as_str()))
        .or_else(|| obj.get("name").and_then(|v| v.as_str()))
        .or_else(|| obj.get("workflowName").and_then(|v| v.as_str()))?
        .to_owned();

    let numeric_id: Option<i64> = obj
        .get("number")
        .and_then(|v| v.as_i64())
        .or_else(|| obj.get("databaseId").and_then(|v| v.as_i64()));

    let status = obj
        .get("state")
        .and_then(|v| v.as_str())
        .or_else(|| obj.get("status").and_then(|v| v.as_str()))
        .or_else(|| obj.get("conclusion").and_then(|v| v.as_str()))
        .map(ToOwned::to_owned);

    let branch = obj
        .get("headBranch")
        .and_then(|v| v.as_str())
        .or_else(|| obj.get("headRefName").and_then(|v| v.as_str()))
        .map(compact_whitespace);

    let comments = extract_comment_count(obj.get("comments"));

    let labels: Vec<String> = obj
        .get("labels")
        .map(extract_label_names)
        .unwrap_or_default()
        .into_iter()
        .take(3)
        .collect();

    let updated_at = obj
        .get("updatedAt")
        .and_then(|v| v.as_str())
        .map(|s| s.get(..10).unwrap_or(s).to_owned());

    let mut parts = Vec::new();
    if let Some(id) = numeric_id {
        parts.push(format!("#{}", id));
    }
    parts.push(compact_whitespace(&title));
    if let Some(s) = status {
        parts.push(format!("[{}]", s));
    }
    if let Some(b) = branch {
        parts.push(format!("({})", b));
    }
    if let Some(c) = comments {
        if c > 0 {
            parts.push(format!("{}c", c));
        }
    }
    if !labels.is_empty() {
        parts.push(format!("{{{}}}", labels.join(", ")));
    }
    if let Some(d) = updated_at {
        parts.push(d);
    }
    Some(parts.join(" "))
}

fn extract_comment_count(value: Option<&serde_json::Value>) -> Option<i64> {
    match value? {
        serde_json::Value::Number(n) => n.as_i64(),
        serde_json::Value::Array(arr) => Some(arr.len() as i64),
        serde_json::Value::Object(obj) => obj.get("totalCount").and_then(|v| v.as_i64()),
        _ => None,
    }
}

fn extract_label_names(value: &serde_json::Value) -> Vec<String> {
    let arr = match value.as_array() {
        Some(a) => a,
        None => return Vec::new(),
    };
    arr.iter()
        .filter_map(|entry| {
            if let Some(s) = entry.as_str() {
                if !s.is_empty() {
                    Some(s.to_owned())
                } else {
                    None
                }
            } else if let Some(obj) = entry.as_object() {
                obj.get("name")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(ToOwned::to_owned)
            } else {
                None
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// JSON pretty-print
// ---------------------------------------------------------------------------

fn pretty_print_json_if_possible(text: &str) -> String {
    let trimmed = text.trim();
    if !(trimmed.starts_with('{') || trimmed.starts_with('[')) {
        return text.to_owned();
    }
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if v.is_object() || v.is_array() {
            return serde_json::to_string_pretty(&v).unwrap_or_else(|_| text.to_owned());
        }
    }
    text.to_owned()
}

// ---------------------------------------------------------------------------
// Raw text builder
// ---------------------------------------------------------------------------

fn build_raw_text(input: &ToolExecutionInput) -> String {
    if let Some(combined) = &input.combined_text {
        return combined.clone();
    }
    let stdout = input.stdout.as_deref().unwrap_or("");
    let stderr = input.stderr.as_deref().unwrap_or("");
    if stdout.is_empty() {
        return stderr.to_owned();
    }
    if stderr.is_empty() {
        return stdout.to_owned();
    }
    format!("{}\n{}", stdout, stderr)
}

// ---------------------------------------------------------------------------
// apply_rule
// ---------------------------------------------------------------------------

struct ApplyResult {
    summary: String,
    facts: HashMap<String, usize>,
}

fn apply_rule(
    compiled_rule: &CompiledRule,
    input: &ToolExecutionInput,
    raw_text: &str,
) -> ApplyResult {
    let rule = &compiled_rule.rule;
    let mut text = raw_text.to_owned();

    if rule
        .transforms
        .as_ref()
        .and_then(|t| t.pretty_print_json)
        .unwrap_or(false)
    {
        text = pretty_print_json_if_possible(&text);
    }

    let mut lines = normalize_lines(&text);
    let mut facts: HashMap<String, usize> = HashMap::new();

    if rule
        .transforms
        .as_ref()
        .and_then(|t| t.strip_ansi)
        .unwrap_or(false)
    {
        lines = normalize_lines(&strip_ansi(&lines.join("\n")));
    }

    // outputMatches check — run on the trimmed full text
    let output_match_text = trim_empty_edges(&lines).join("\n");
    if let Some(matched_output) = compiled_rule
        .compiled
        .output_matches
        .iter()
        .find(|entry| entry.pattern.is_match(&output_match_text))
    {
        return ApplyResult {
            summary: matched_output.message.clone(),
            facts,
        };
    }

    // skipPatterns
    if rule
        .filters
        .as_ref()
        .and_then(|f| f.skip_patterns.as_ref())
        .map(|p| !p.is_empty())
        .unwrap_or(false)
    {
        lines.retain(|line| {
            !compiled_rule
                .compiled
                .skip_patterns
                .iter()
                .any(|pat| pat.is_match(line))
        });
    }

    // counter_source == preKeep → sample counters before keep filtering
    let pre_keep_lines = lines.clone();

    // keepPatterns
    let has_keep = !compiled_rule.compiled.keep_patterns.is_empty();
    if has_keep {
        let kept: Vec<String> = lines
            .iter()
            .filter(|line| {
                compiled_rule
                    .compiled
                    .keep_patterns
                    .iter()
                    .any(|pat| pat.is_match(line))
            })
            .cloned()
            .collect();
        if !kept.is_empty() {
            lines = kept;
        }
    }

    // trimEmptyEdges
    if rule
        .transforms
        .as_ref()
        .and_then(|t| t.trim_empty_edges)
        .unwrap_or(false)
    {
        lines = trim_empty_edges(&lines);
    }

    // dedupeAdjacent
    if rule
        .transforms
        .as_ref()
        .and_then(|t| t.dedupe_adjacent)
        .unwrap_or(false)
    {
        lines = dedupe_adjacent(&lines);
    }

    // Special post-processors
    if rule.id == "git/status" {
        lines = rewrite_git_status_lines(&lines);
    }
    if rule.id == "cloud/gh" {
        lines = rewrite_gh_lines(&lines, input);
    }

    // Counters
    let counter_lines = match &rule.counter_source {
        Some(CounterSource::PreKeep) => &pre_keep_lines,
        _ => &lines,
    };
    for counter in &compiled_rule.compiled.counters {
        let count = counter_lines
            .iter()
            .filter(|line| counter.pattern.is_match(line))
            .count();
        facts.insert(counter.name.clone(), count);
    }

    // onEmpty
    if lines.is_empty() {
        if let Some(on_empty) = &rule.on_empty {
            return ApplyResult {
                summary: on_empty.clone(),
                facts,
            };
        }
    }

    // Failure-preserving summarize
    let is_failure = input.exit_code.map(|c| c != 0).unwrap_or(false);
    let preserve_on_failure = rule
        .failure
        .as_ref()
        .and_then(|f| f.preserve_on_failure)
        .unwrap_or(false);

    let (head, tail) = if is_failure && preserve_on_failure {
        (
            rule.failure.as_ref().and_then(|f| f.head).unwrap_or(6),
            rule.failure.as_ref().and_then(|f| f.tail).unwrap_or(12),
        )
    } else {
        (
            rule.summarize.as_ref().and_then(|s| s.head).unwrap_or(6),
            rule.summarize.as_ref().and_then(|s| s.tail).unwrap_or(6),
        )
    };

    log::debug!(
        "[tokenjuice] apply_rule '{}': {} lines → head={} tail={} failure={}",
        rule.id,
        lines.len(),
        head,
        tail,
        is_failure && preserve_on_failure
    );

    let compacted = head_tail(&lines, head, tail);
    ApplyResult {
        summary: compacted.join("\n").trim().to_owned(),
        facts,
    }
}

// ---------------------------------------------------------------------------
// Passthrough text
// ---------------------------------------------------------------------------

fn build_passthrough_text(input: &ToolExecutionInput, raw_text: &str) -> String {
    let normalized = trim_empty_edges(&normalize_lines(&strip_ansi(raw_text)))
        .join("\n")
        .trim()
        .to_owned();
    if normalized.is_empty() {
        return "(no output)".to_owned();
    }
    if input.exit_code.map(|c| c != 0).unwrap_or(false) {
        return format!("exit {}\n{}", input.exit_code.unwrap(), normalized);
    }
    normalized
}

// ---------------------------------------------------------------------------
// format_inline
// ---------------------------------------------------------------------------

fn format_inline(
    classification: &ClassificationResult,
    input: &ToolExecutionInput,
    summary: &str,
    facts: &HashMap<String, usize>,
) -> String {
    let mut fact_parts: Vec<String> = facts
        .iter()
        .filter(|(_, &count)| count > 0)
        .map(|(name, &count)| pluralize(count, name))
        .collect();
    fact_parts.sort_unstable();

    let mut lines: Vec<String> = Vec::new();
    if input.exit_code.map(|c| c != 0).unwrap_or(false) {
        lines.push(format!("exit {}", input.exit_code.unwrap()));
    }

    let include_facts = classification.family == "search"
        || (classification.family != "git-status"
            && classification.family != "help"
            && summary.contains("omitted"))
        || (classification.family == "test-results"
            && input.exit_code.map(|c| c != 0).unwrap_or(false));

    if include_facts && !fact_parts.is_empty() {
        lines.push(fact_parts.join(", "));
    }
    lines.push(summary.to_owned());
    lines.join("\n").trim().to_owned()
}

// ---------------------------------------------------------------------------
// select_inline_text
// ---------------------------------------------------------------------------

fn select_inline_text(
    classification: &ClassificationResult,
    input: &ToolExecutionInput,
    raw_text: &str,
    compact_text: &str,
    max_inline_chars: usize,
) -> String {
    if classification.family == "git-status" {
        return compact_text.to_owned();
    }

    let passthrough = build_passthrough_text(input, raw_text);
    let raw_chars = count_text_chars(&strip_ansi(raw_text));
    let compact_chars = count_text_chars(compact_text);
    let passthrough_limit = if classification.family == "help" {
        max_inline_chars
    } else {
        TINY_OUTPUT_MAX_CHARS
    };

    if count_text_chars(&passthrough) > passthrough_limit {
        return compact_text.to_owned();
    }
    if raw_chars <= max_inline_chars && compact_chars >= raw_chars {
        return passthrough;
    }
    if count_text_chars(&passthrough) <= compact_chars {
        return passthrough;
    }
    compact_text.to_owned()
}

// ---------------------------------------------------------------------------
// reduce_execution_with_rules  (sync, library-only)
// ---------------------------------------------------------------------------

/// Reduce `input` using a pre-loaded set of compiled rules.
///
/// This is the synchronous, library-only entry point (no async, no artifact
/// store — those are deferred to v2).
pub fn reduce_execution_with_rules(
    input: ToolExecutionInput,
    rules: &[CompiledRule],
    opts: &ReduceOptions,
) -> CompactResult {
    let normalized_input = normalize_execution_input(input);
    let raw_text = build_raw_text(&normalized_input);
    let measured_raw_chars = count_text_chars(&strip_ansi(&raw_text));
    let classification = classify_execution(&normalized_input, rules, opts.classifier.as_deref());

    log::debug!(
        "[tokenjuice] reduce_execution: tool='{}' raw_chars={} family='{}'",
        normalized_input.tool_name,
        measured_raw_chars,
        classification.family
    );

    // raw pass-through mode
    if opts.raw.unwrap_or(false) {
        return CompactResult {
            inline_text: raw_text,
            preview_text: None,
            facts: None,
            stats: ReductionStats {
                raw_chars: measured_raw_chars,
                reduced_chars: measured_raw_chars,
                ratio: 1.0,
            },
            classification,
        };
    }

    // File-content inspection commands are never compacted
    if classification.matched_reducer.as_deref() == Some("generic/fallback")
        && is_file_content_inspection_command(&normalized_input)
    {
        return CompactResult {
            inline_text: raw_text,
            preview_text: None,
            facts: None,
            stats: ReductionStats {
                raw_chars: measured_raw_chars,
                reduced_chars: measured_raw_chars,
                ratio: 1.0,
            },
            classification,
        };
    }

    // Find the matched rule (fall back to generic/fallback)
    let matched_rule = rules
        .iter()
        .find(|r| Some(r.rule.id.as_str()) == classification.matched_reducer.as_deref())
        .or_else(|| rules.iter().find(|r| r.rule.id == "generic/fallback"))
        .expect("generic/fallback rule must be present in the rule set");

    let ApplyResult { summary, facts } = apply_rule(matched_rule, &normalized_input, &raw_text);

    let compact_text = format_inline(
        &classification,
        &normalized_input,
        &summary.or_empty(),
        &facts,
    );

    let max_inline_chars = opts.max_inline_chars.unwrap_or(1200);
    let selected = select_inline_text(
        &classification,
        &normalized_input,
        &raw_text,
        &compact_text,
        max_inline_chars,
    );

    let use_middle_clamp = classification.family == "help" || selected.contains('\n');
    let inline_text = if use_middle_clamp {
        clamp_text_middle(&selected, max_inline_chars)
    } else {
        clamp_text(&selected, max_inline_chars)
    };

    let reduced_chars = count_text_chars(&inline_text);
    let ratio = if measured_raw_chars == 0 {
        1.0
    } else {
        reduced_chars as f64 / measured_raw_chars as f64
    };

    log::debug!(
        "[tokenjuice] reduce_execution complete: rule='{}' raw={} reduced={} ratio={:.2}",
        classification.matched_reducer.as_deref().unwrap_or("?"),
        measured_raw_chars,
        reduced_chars,
        ratio
    );

    CompactResult {
        inline_text,
        preview_text: if summary.is_empty() {
            None
        } else {
            Some(summary)
        },
        facts: if facts.is_empty() { None } else { Some(facts) },
        stats: ReductionStats {
            raw_chars: measured_raw_chars,
            reduced_chars,
            ratio,
        },
        classification,
    }
}

// ---------------------------------------------------------------------------
// Convenience trait
// ---------------------------------------------------------------------------

trait OrEmpty {
    fn or_empty(&self) -> String;
}
impl OrEmpty for String {
    fn or_empty(&self) -> String {
        if self.is_empty() {
            "(no output)".to_owned()
        } else {
            self.clone()
        }
    }
}

// ---------------------------------------------------------------------------
// Regex helpers (avoid repeated compilation)
// ---------------------------------------------------------------------------

fn regex_match(pattern: &str, text: &str) -> bool {
    regex::Regex::new(pattern)
        .map(|re| re.is_match(text))
        .unwrap_or(false)
}

fn regex_replace(pattern: &str, text: &str, replacement: &str) -> String {
    regex::Regex::new(pattern)
        .map(|re| re.replace(text, replacement).into_owned())
        .unwrap_or_else(|_| text.to_owned())
}

fn regex_captures(pattern: &str, text: &str) -> Option<Vec<String>> {
    let re = regex::Regex::new(pattern).ok()?;
    let caps = re.captures(text)?;
    Some(
        (1..caps.len())
            .filter_map(|i| caps.get(i).map(|m| m.as_str().to_owned()))
            .collect(),
    )
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "reduce_tests.rs"]
mod tests;
