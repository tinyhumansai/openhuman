// Exact host SDK contracts; never general runtime/harness forwarding.
const compact = (value) => value.replace(/\s+/g, "").replace(/,}/g, "}");
const contracts = new Map([
  ["crates/openhuman-core/src/agent/tinyagents/budget.rs", compact(
    "pub use tinyinference_llm::model::budget::{Budget, BudgetExceeded, BudgetSnapshot, CallBudget, Spend, SpendLimits};",
  )],
  ["crates/openhuman-core/src/agent/tinyagents/turn_observer.rs", compact(
    "pub use tinyagents_harness::observability::{trace_export::{otlp::otlp_requests, ExportBrand, SpanKind, SpanStatus, TraceSpan}, LangfuseAuth, LangfuseClient, LangfuseScore, LangfuseScoreValue, LangfuseTraceConfig};",
  )],
]);

// Callers pass Rust with comments/literals masked. Full-statement matching
// prevents adding a runtime type to a multiline approved forwarding statement.
export function sanctionedSdkReexportLines(path, code) {
  const approved = contracts.get(path);
  const lines = new Set();
  if (!approved) return lines;
  for (const match of code.matchAll(/\bpub\s+use\s+[^;]+;/g)) {
    if (compact(match[0]) === approved) {
      lines.add(code.slice(0, match.index).split(/\r?\n/).length);
    }
  }
  return lines;
}
