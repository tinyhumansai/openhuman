import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import test from "node:test";

test("core chat failure keys are used even though the UI translates them dynamically", () => {
  const result = spawnSync(
    "pnpm",
    ["exec", "tsx", "scripts/i18n-coverage.ts", "--json"],
    { encoding: "utf8" },
  );
  assert.equal(result.status, 0, result.stderr);
  const report = JSON.parse(result.stdout);
  const table = readFileSync(
    "crates/openhuman-core/src/inference/failure_copy/table.rs",
    "utf8",
  );
  const keys = [...table.matchAll(/"(chat_error\.[a-z_]+)"/g)].map(
    (match) => match[1],
  );
  assert.ok(keys.length >= 28);
  assert.deepEqual(
    keys.filter((key) => report.unusedKeys.includes(key)),
    [],
  );
  assert.ok(
    !report.unusedKeys.includes("keyring.settings.mode.consentPending"),
  );
  assert.ok(
    !report.unusedKeys.includes("settings.gateway.destinationRequired"),
  );
  assert.ok(
    !report.unusedKeys.includes(
      "devOptions.toolPolicyDiagnostics.inventory.totalTools",
    ),
  );
  assert.ok(!report.unusedKeys.includes("conversations.tools.readFile.active"));
  assert.ok(!report.unusedKeys.includes("flows.templates.webhook-triage.name"));
});
