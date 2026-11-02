import test from "node:test";
import assert from "node:assert/strict";
import {
  copyFileSync,
  mkdirSync,
  mkdtempSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, resolve } from "node:path";
import { spawnSync } from "node:child_process";

function check(files) {
  const root = mkdtempSync(resolve(tmpdir(), "embed-contract-boundary-"));
  try {
    mkdirSync(resolve(root, "crates"), { recursive: true });
    mkdirSync(resolve(root, "vendor/tinyagents/crates"), { recursive: true });
    mkdirSync(resolve(root, "scripts/ci"), { recursive: true });
    mkdirSync(resolve(root, "scripts/lib"), { recursive: true });
    const bridge =
      "vendor/tinyagents/crates/tinyagents-harness/src/providers/claude_code/bridge.rs";
    mkdirSync(dirname(resolve(root, bridge)), { recursive: true });
    writeFileSync(
      resolve(root, bridge),
      "struct ChatMessage { pub(crate) role: String, pub(crate) content: String, }\n",
    );
    copyFileSync(
      new URL("../ci/check-agent-runtime-boundary.mjs", import.meta.url),
      resolve(root, "scripts/ci/check-agent-runtime-boundary.mjs"),
    );
    for (const helper of [
      "runtime-boundary-types.mjs",
      "agent-sdk-contracts.mjs",
    ]) {
      copyFileSync(
        new URL(`../lib/${helper}`, import.meta.url),
        resolve(root, `scripts/lib/${helper}`),
      );
    }
    writeFileSync(
      resolve(root, "scripts/ci/agent-runtime-boundary-baseline.json"),
      "[]\n",
    );
    for (const [path, source] of Object.entries(files)) {
      mkdirSync(dirname(resolve(root, path)), { recursive: true });
      writeFileSync(resolve(root, path), source);
    }
    const result = spawnSync(
      process.execPath,
      [resolve(root, "scripts/ci/check-agent-runtime-boundary.mjs")],
      { encoding: "utf8" },
    );
    assert.equal(result.error, undefined);
    return { status: result.status, output: result.stdout + result.stderr };
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

const contracts = {
  "crates/openhuman-embed/src/stream.rs":
    "pub use tinyagents_harness::cancel::CancellationToken;\n",
  "crates/openhuman-embed/src/config.rs":
    "pub use tinytools::{\n DefaultEffect, Patterns, RuleEffect, Surface, ToolMatcher, ToolRule, ToolRules,\n};\n",
  "crates/openhuman-embed/src/lib.rs":
    "pub mod providers {\n pub use tinyinference_llm::message::MessageDelta;\n pub use tinyinference_llm::model::{\n ChatModel, DeferredHandle, DeferredStatus, ModelProfile, ModelRequest, ModelResponse,\n ModelStream, ModelStreamItem, ModelStreamMetadata,\n };\n pub use tinyinference_llm::{Error, Result};\n}\n",
};

test("Embed exposes the native provider and policy contracts without copying their owners", () => {
  const result = check(contracts);
  assert.equal(result.status, 0, result.output);
});

test("the public contract allowance does not admit another export in an approved file", () => {
  const result = check({
    ...contracts,
    "crates/openhuman-embed/src/lib.rs": contracts[
      "crates/openhuman-embed/src/lib.rs"
    ].replace("ModelStreamMetadata,", "ModelStreamMetadata, UnreviewedType,"),
  });
  assert.equal(result.status, 1, result.output);
  assert.match(result.output, /openhuman-upstream-reexport/);
});

test("the same native contract remains forbidden outside its owning adapter", () => {
  const result = check({
    "crates/openhuman-core/src/other/mod.rs":
      contracts["crates/openhuman-embed/src/stream.rs"],
  });
  assert.equal(result.status, 1, result.output);
  assert.match(result.output, /openhuman-upstream-reexport/);
});

test("Embed preserves its existing native tool, session and budget injection contracts", () => {
  const result = check({
    "crates/openhuman-embed/src/lib.rs":
      "pub use tinytools::{PermissionLevel, Tool, ToolExposure, ToolResult};\n" +
      "pub use tinyagents_session::port::{AgentStores, SessionStoreProvider};\n",
    "crates/openhuman-embed/src/budget.rs":
      "pub use tinyinference_llm::model::budget::{Budget, BudgetExceeded, BudgetSnapshot, CallBudget, Spend, SpendLimits};\n",
  });
  assert.equal(result.status, 0, result.output);
});

test("native injection allowances reject expanded or core compatibility exports", () => {
  for (const files of [
    {
      "crates/openhuman-core/src/agent/host_overrides.rs":
        "pub use tinyagents_harness::cancel::CancellationToken;\n",
    },
    {
      "crates/openhuman-embed/src/lib.rs":
        "pub use tinytools::{PermissionLevel, Tool, ToolExposure, ToolResult, ToolRegistry};\n",
    },
    {
      "crates/openhuman-core/src/tools/mod.rs":
        "pub use tinytools::{PermissionLevel, Tool, ToolExposure, ToolResult};\n",
    },
    {
      "crates/openhuman-core/src/storage/sessions/mod.rs":
        "pub use tinyagents_session::port::{AgentStores, SessionStoreProvider};\n",
    },
    {
      "crates/openhuman-embed/src/budget.rs":
        "pub use tinyinference_llm::model::budget::{Budget, BudgetExceeded, BudgetSnapshot, CallBudget, Spend, SpendLimits, BudgetedModel};\n",
    },
  ]) {
    const result = check(files);
    assert.equal(result.status, 1, result.output);
    assert.match(result.output, /openhuman-upstream-reexport/);
  }
});

test("Embed preserves native persisted codecs without admitting core facades", () => {
  const codecs = [
    "pub use tinyagents_harness_tools::todos::TodoList;",
    "pub use tinyagents_harness::events::{AgentEvent, HarnessRunStatus};",
    "pub use tinyagents_harness::ids::ExecutionStatus;",
    "pub use tinyagents_harness::observability::AgentObservation;",
    "pub use tinyagents_session::transcript::import::types::{JournalMessage, SessionDescriptor};",
  ].join("\n");
  const allowed = check({ "crates/openhuman-embed/src/lib.rs": codecs });
  assert.equal(allowed.status, 0, allowed.output);
  for (const files of [
    { "crates/openhuman-core/src/storage/sessions/mod.rs": codecs },
    {
      "crates/openhuman-embed/src/lib.rs": codecs.replace(
        "AgentEvent, HarnessRunStatus",
        "AgentEvent, HarnessRunStatus, UnreviewedType",
      ),
    },
  ]) {
    const rejected = check(files);
    assert.equal(rejected.status, 1, rejected.output);
    assert.match(rejected.output, /openhuman-upstream-reexport/);
  }
});

test("a builder method named with_thread_id is not a task-local accessor", () => {
  const result = check({
    "crates/openhuman-tinyhumans/src/inference/managed_model.rs":
      "let model = builder.with_thread_id(thread_id);\n",
  });
  assert.equal(result.status, 0, result.output);
});

test("task-local with_thread_id calls, imports, and declarations remain forbidden", () => {
  for (const source of [
    "let request = with_thread_id(request, thread_id);\n",
    "use crate::turn_context::with_thread_id;\n",
    "fn with_thread_id(request: Request) -> Request { request }\n",
  ]) {
    const result = check({ "crates/openhuman-core/src/example.rs": source });
    assert.equal(result.status, 1, result.output);
    assert.match(result.output, /openhuman-task-local/);
  }
});

test("Embed preserves the exact native dispatch-budget and observer contracts", () => {
  const budget =
    "pub use tinyagents_harness::cost::{\n" +
    " ControlledDispatchError, DispatchModelOptions, DispatchRefusal, DispatchRefusalReason,\n" +
    " PhysicalDispatchAdmission, PhysicalDispatchPermit,\n" +
    "};\n";
  const observer =
    "pub use tinyagents_harness::observability::{\n" +
    " PhysicalDispatchMetadata, PhysicalDispatchObserver, PhysicalDispatchOutcome,\n" +
    " PhysicalDispatchStatus, PhysicalDispatchTerminal, PhysicalDispatchUsage,\n" +
    "};\n";
  const allowed = check({
    "crates/openhuman-embed/src/budget.rs": budget,
    "crates/openhuman-embed/src/complete.rs": observer,
  });
  assert.equal(allowed.status, 0, allowed.output);

  for (const files of [
    {
      "crates/openhuman-embed/src/budget.rs": budget.replace(
        "PhysicalDispatchAdmission, PhysicalDispatchPermit,",
        "PhysicalDispatchAdmission, PhysicalDispatchPermit, UnreviewedType,",
      ),
    },
    {
      "crates/openhuman-embed/src/complete.rs": observer.replace(
        "PhysicalDispatchStatus, PhysicalDispatchTerminal, PhysicalDispatchUsage,",
        "PhysicalDispatchStatus, PhysicalDispatchTerminal, PhysicalDispatchUsage, UnreviewedType,",
      ),
    },
    { "crates/openhuman-core/src/agent/tinyagents/budget.rs": budget },
  ]) {
    const rejected = check(files);
    assert.equal(rejected.status, 1, rejected.output);
    assert.match(rejected.output, /openhuman-upstream-reexport/);
  }
});

test("Embed exposes exact native provider override contracts in inference_registry", () => {
  const overrides =
    "pub mod inference_registry {\n" +
    " pub use tinyinference_llm::providers::overrides::{\n" +
    "  BoxFuture, CatalogEntry, CatalogLimits, CatalogListing, CatalogRequest,\n" +
    "  ChatModelEntry, ChatModelFactory, ChatModelRequest, EmbeddingFactory,\n" +
    "  EmbeddingRequest, ProviderModelMetadata, ProviderOverrides, ReadinessProbe,\n" +
    " };\n" +
    "}\n";
  const allowed = check({
    "crates/openhuman-embed/src/host_internals.rs": overrides,
  });
  assert.equal(allowed.status, 0, allowed.output);

  for (const files of [
    {
      "crates/openhuman-embed/src/host_internals.rs": overrides.replace(
        "ProviderOverrides, ReadinessProbe,",
        "ProviderOverrides, ReadinessProbe, UnreviewedType,",
      ),
    },
    {
      "crates/openhuman-embed/src/host_internals.rs": overrides.replace(
        "inference_registry",
        "provider_registry",
      ),
    },
    {
      "crates/openhuman-core/src/inference/registry.rs": overrides,
    },
  ]) {
    const rejected = check(files);
    assert.equal(rejected.status, 1, rejected.output);
    assert.match(rejected.output, /openhuman-upstream-reexport/);
  }
});

test("the TinyAgents-owned AgentProgress type is allowed only at its native paths", () => {
  const progressOwner =
    "vendor/tinyagents/crates/tinyagents-harness/src/progress.rs";
  const harness = "vendor/tinyagents/crates/tinyagents-harness/src/";
  const native = {
    [`${harness}lib.rs`]: "pub use progress::AgentProgress;\n",
    [progressOwner]: "pub enum AgentProgress { Started }\n",
    [`${harness}turn_runner/types.rs`]:
      "use crate::progress::AgentProgress;\nstruct Event(AgentProgress);\n",
    [`${harness}turn_runner/mod_tests.rs`]:
      "fn progress(event: crate::progress::AgentProgress) { let _ = event; }\n",
  };
  const allowed = check(native);
  assert.equal(allowed.status, 0, allowed.output);

  for (const files of [
    {
      "vendor/tinyagents/crates/tinyagents-session/src/progress.rs":
        "pub enum AgentProgress { Started }\n",
    },
    {
      "vendor/tinyagents/crates/tinyagents-harness/src/agent_progress.rs":
        "pub enum AgentProgress { Started }\n",
    },
    {
      [progressOwner]:
        "pub enum AgentProgress { Started }\nuse openhuman_core::AgentProgress;\n",
    },
    {
      [`${harness}turn_runner/types.rs`]:
        "use crate::progress::AgentProgress;\nuse openhuman_core::AgentProgress;\n",
    },
    {
      [`${harness}turn_runner/types.rs`]:
        "/* use crate::progress::AgentProgress; */\nstruct Event(AgentProgress);\n",
    },
    {
      "vendor/tinyagents/crates/tinyagents-session/src/lib.rs":
        "pub enum AgentProgress { Started }\n",
    },
  ]) {
    const rejected = check(files);
    assert.equal(rejected.status, 1, rejected.output);
    assert.match(rejected.output, /tinyagents-openhuman-domain-name/);
  }
});

test("provider override contract does not escape its closed facade module", () => {
  const overrides =
    "pub mod inference_registry {\n" +
    "}\n" +
    "pub mod other_registry {\n" +
    " pub use tinyinference_llm::providers::overrides::{\n" +
    "  BoxFuture, CatalogEntry, CatalogLimits, CatalogListing, CatalogRequest,\n" +
    "  ChatModelEntry, ChatModelFactory, ChatModelRequest, EmbeddingFactory,\n" +
    "  EmbeddingRequest, ProviderModelMetadata, ProviderOverrides, ReadinessProbe,\n" +
    " };\n" +
    "}\n";
  const misplaced = check({
    "crates/openhuman-embed/src/host_internals.rs": overrides,
  });
  assert.equal(misplaced.status, 1, misplaced.output);
  assert.match(misplaced.output, /openhuman-upstream-reexport/);

  const freeExport = check({
    "crates/openhuman-embed/src/host_internals.rs":
      "pub mod inference_registry {\n}\n" +
      "pub use tinyinference_llm::providers::overrides::{\n" +
      " BoxFuture, CatalogEntry, CatalogLimits, CatalogListing, CatalogRequest,\n" +
      " ChatModelEntry, ChatModelFactory, ChatModelRequest, EmbeddingFactory,\n" +
      " EmbeddingRequest, ProviderModelMetadata, ProviderOverrides, ReadinessProbe,\n" +
      "};\n",
  });
  assert.equal(freeExport.status, 1, freeExport.output);
  assert.match(freeExport.output, /openhuman-upstream-reexport/);

  const nestedExport = check({
    "crates/openhuman-embed/src/host_internals.rs":
      "pub mod inference_registry {\n" +
      " pub mod nested {\n" +
      "  pub use tinyinference_llm::providers::overrides::{\n" +
      "   BoxFuture, CatalogEntry, CatalogLimits, CatalogListing, CatalogRequest,\n" +
      "   ChatModelEntry, ChatModelFactory, ChatModelRequest, EmbeddingFactory,\n" +
      "   EmbeddingRequest, ProviderModelMetadata, ProviderOverrides, ReadinessProbe,\n" +
      "  };\n" +
      " }\n" +
      "}\n",
  });
  assert.equal(nestedExport.status, 1, nestedExport.output);
  assert.match(nestedExport.output, /openhuman-upstream-reexport/);
});
