import test from "node:test";
import assert from "node:assert/strict";

import {
  SCENARIOS,
  EXPECTED_CONTEXT_WINDOW,
  createMockLocalLlmServer,
  executeScenario,
  runMatrix,
  generateMarkdownEvidence,
} from "./local-qwen-phase7-live.mjs";

test("matrix defines all 12 Phase 7 representative acceptance scenarios", () => {
  assert.equal(SCENARIOS.length, 12);

  const scenario1 = SCENARIOS.find((s) => s.id === 1);
  assert.equal(scenario1.mode, "chat");
  assert.equal(scenario1.expectedToolsCount, 0);
  assert.equal(scenario1.maxCalls, 1);
  assert.equal(scenario1.expectedStopReason, "final_answer");

  const scenario2 = SCENARIOS.find((s) => s.id === 2);
  assert.equal(scenario2.mode, "chat");
  assert.equal(scenario2.expectedToolsCount, 0);
  assert.equal(scenario2.maxCalls, 1);

  const scenario3 = SCENARIOS.find((s) => s.id === 3);
  assert.equal(scenario3.mode, "assist");
  assert.equal(scenario3.allowMeteredTools, false);
  assert.ok(scenario3.maxCalls <= 3);

  const scenario4 = SCENARIOS.find((s) => s.id === 4);
  assert.equal(scenario4.mode, "assist");
  assert.ok(scenario4.description.includes("fetch first"));

  const scenario5 = SCENARIOS.find((s) => s.id === 5);
  assert.equal(scenario5.mode, "assist");
  assert.ok(scenario5.description.includes("retrieval only"));

  const scenario6 = SCENARIOS.find((s) => s.id === 6);
  assert.equal(scenario6.mode, "assist");
  assert.equal(scenario6.allowMeteredTools, false);
  assert.equal(scenario6.expectedStopReason, "unavailable");

  const scenario7 = SCENARIOS.find((s) => s.id === 7);
  assert.equal(scenario7.mode, "assist");
  assert.equal(scenario7.allowMeteredTools, true);
  assert.equal(scenario7.expectedStopReason, "repeated_failure");

  const scenario8 = SCENARIOS.find((s) => s.id === 8);
  assert.equal(scenario8.mode, "chat");
  assert.equal(scenario8.expectedToolsCount, 0);

  const scenario9 = SCENARIOS.find((s) => s.id === 9);
  assert.equal(scenario9.mode, "assist");
  assert.equal(scenario9.simulateFailedMemory, true);

  const scenario10 = SCENARIOS.find((s) => s.id === 10);
  assert.equal(scenario10.mode, "agent");
  assert.ok(scenario10.expectedToolsCount > 1);

  const scenario11 = SCENARIOS.find((s) => s.id === 11);
  assert.equal(scenario11.mode, "assist");
  assert.equal(scenario11.simulateMalformedQwenCall, true);

  const scenario12 = SCENARIOS.find((s) => s.id === 12);
  assert.equal(scenario12.mode, "agent");
  assert.equal(scenario12.simulateInterruption, true);
});

test("mock local server provides models and chat completion endpoints", async () => {
  const mockServer = createMockLocalLlmServer();
  const endpoint = await mockServer.listen(0);

  try {
    const modelsRes = await fetch(`${endpoint}/models`);
    assert.equal(modelsRes.status, 200);
    const modelsData = await modelsRes.json();
    assert.equal(modelsData.data[0].id, "lmstudio:qwen38-openhuman");
    assert.equal(modelsData.data[0].context_length, EXPECTED_CONTEXT_WINDOW);

    const chatRes = await fetch(`${endpoint}/chat/completions`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        model: "lmstudio:qwen38-openhuman",
        messages: [{ role: "user", content: "hey" }],
      }),
    });
    assert.equal(chatRes.status, 200);
    const chatData = await chatRes.json();
    assert.ok(chatData.choices[0].message.content.includes("Hello"));
    assert.equal(mockServer.endpointCounters.chatCompletions, 1);
  } finally {
    await mockServer.close();
  }
});

test("executeScenario validates invariants for greetings and disabled metered tool", async () => {
  const mockServer = createMockLocalLlmServer();
  const endpoint = await mockServer.listen(0);

  try {
    const scenario1 = SCENARIOS[0];
    const res1 = await executeScenario(scenario1, { endpoint });
    assert.equal(res1.passed, true);
    assert.equal(res1.primaryCalls, 1);
    assert.equal(res1.advertisedTools.length, 0);
    assert.equal(res1.rawMarkupFound, false);
    assert.equal(res1.contextWindow, 131072);

    const scenario6 = SCENARIOS[5];
    const res6 = await executeScenario(scenario6, { endpoint });
    assert.equal(res6.passed, true);
    assert.equal(res6.stopReason, "unavailable");
    assert.equal(res6.advertisedTools.length, 0);
    assert.equal(res6.primaryCalls, 0);
  } finally {
    await mockServer.close();
  }
});

test("runMatrix executes full suite and produces complete JSON and Markdown evidence", async () => {
  const mockServer = createMockLocalLlmServer();
  const endpoint = await mockServer.listen(0);

  try {
    const matrixResult = await runMatrix({ endpoint });
    assert.equal(matrixResult.allPassed, true);
    assert.equal(matrixResult.results.length, 12);
    assert.equal(matrixResult.contextWindow, 131072);

    const md = matrixResult.markdownEvidence;
    assert.ok(md.includes("# Phase 7 Local-Qwen Live Acceptance Matrix Evidence"));
    assert.ok(md.includes("**Resolved Context Window**: `131072`"));
    assert.ok(md.includes("**Status**: PASSED (12/12)"));
    assert.ok(md.includes("greetings_chat"));
    assert.ok(md.includes("interruption_and_resume"));
    assert.ok(md.includes("Image retrieval enforces modality boundary"));
  } finally {
    await mockServer.close();
  }
});
