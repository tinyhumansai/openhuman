import assert from "node:assert/strict";
import { closeSync, mkdtempSync, openSync, readFileSync, rmSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

test("core chat failure keys are used even though the UI translates them dynamically", () => {
  const directory = mkdtempSync(join(tmpdir(), "i18n-coverage-"));
  const reportPath = join(directory, "report.json");
  const reportFd = openSync(reportPath, "w");
  let report;
  try {
    const result = spawnSync(
      process.execPath,
      ["node_modules/tsx/dist/cli.mjs", "scripts/i18n-coverage.ts", "--json"],
      { encoding: "utf8", maxBuffer: 1024 * 1024, stdio: ["ignore", reportFd, "pipe"] },
    );
    assert.equal(result.status, 0, result.stderr);
    report = JSON.parse(readFileSync(reportPath, "utf8"));
  } finally {
    closeSync(reportFd);
    rmSync(directory, { recursive: true, force: true });
  }
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
