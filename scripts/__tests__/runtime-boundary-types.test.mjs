import assert from "node:assert/strict";
import test from "node:test";
import { isClaudeCodeBridgeMessage } from "../lib/runtime-boundary-types.mjs";

const root =
  "vendor/tinyagents/crates/tinyagents-harness/src/providers/claude_code/";
const neutral =
  "pub(crate) struct ChatMessage { pub(crate) role: String, pub(crate) content: String, }";

test("recognizes only the reviewed neutral provider message", () => {
  assert.equal(
    isClaudeCodeBridgeMessage(
      `${root}input_builder.rs`,
      "ChatMessage",
      neutral,
    ),
    true,
  );
  assert.equal(
    isClaudeCodeBridgeMessage(`${root}driver_tests.rs`, "ChatMessage", neutral),
    true,
  );
});

test("product record fields cannot inherit the provider exception", () => {
  const product = neutral.replace(
    "content: String,",
    "content: String, pub(crate) thread_id: String,",
  );
  assert.equal(
    isClaudeCodeBridgeMessage(
      `${root}input_builder.rs`,
      "ChatMessage",
      product,
    ),
    false,
  );
  assert.equal(
    isClaudeCodeBridgeMessage(
      `${root}input_builder.rs`,
      "AgentProgress",
      neutral,
    ),
    false,
  );
});

test("other crates and provider modules keep the domain-type rule", () => {
  assert.equal(
    isClaudeCodeBridgeMessage(
      "vendor/tinyagents/crates/tinyagents-session/src/mod.rs",
      "ChatMessage",
      neutral,
    ),
    false,
  );
  assert.equal(
    isClaudeCodeBridgeMessage(
      root.replace("claude_code/", "claude_agent_sdk/") + "mod.rs",
      "ChatMessage",
      neutral,
    ),
    false,
  );
});

test("qualified product imports are never treated as the neutral DTO", () => {
  assert.equal(
    isClaudeCodeBridgeMessage(
      `${root}input_builder.rs`,
      "ChatMessage",
      neutral,
      "use openhuman_core::ChatMessage;",
    ),
    false,
  );
});
