import assert from "node:assert/strict";
import test from "node:test";
import { sanctionedSdkReexportLines } from "../lib/agent-sdk-contracts.mjs";

const budgetPath = "crates/openhuman-core/src/agent/tinyagents/budget.rs";
const observerPath = "crates/openhuman-core/src/agent/tinyagents/turn_observer.rs";
const budget = "pub use tinyinference_llm::model::budget::{\n Budget, BudgetExceeded, BudgetSnapshot, CallBudget, Spend, SpendLimits,\n};";
const transport = "pub use tinyagents_harness::observability::{\n trace_export::{otlp::otlp_requests, ExportBrand, SpanKind, SpanStatus, TraceSpan},\n LangfuseAuth, LangfuseClient, LangfuseScore, LangfuseScoreValue, LangfuseTraceConfig,\n};";

test("native ledger and existing telemetry transport have exact host contracts", () => {
  assert.deepEqual([...sanctionedSdkReexportLines(budgetPath, `\n${budget}`)], [2]);
  assert.deepEqual([...sanctionedSdkReexportLines(observerPath, transport)], [1]);
});

test("inventory refuses runtime types, wildcard, aliases and physically moved contracts", () => {
  for (const statement of [
    budget.replace("SpendLimits,", "SpendLimits, BudgetedModel,"),
    budget.replace("Budget,", "Budget as HostBudget,"),
    "pub use tinyinference_llm::model::budget::*;",
    transport.replace("LangfuseAuth,", "AgentHarness, LangfuseAuth,"),
    transport.replace("TraceSpan}", "TraceSpan, TraceContext}"),
    "pub use tinyagents_harness::observability::{LangfuseAuth, LangfuseClient, LangfuseScore, LangfuseScoreValue, LangfuseTraceConfig};",
    "pub use tinyagents_harness::events::AgentEvent;",
  ]) {
    assert.equal(sanctionedSdkReexportLines(budgetPath, statement).size, 0);
    assert.equal(sanctionedSdkReexportLines(observerPath, statement).size, 0);
  }
  assert.equal(sanctionedSdkReexportLines("crates/openhuman-embed/src/budget.rs", budget).size, 0);
});
