import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const script = join(
  dirname(fileURLToPath(import.meta.url)),
  "..",
  "ci-cancel-aware.sh",
);

// Arm the watchdog exactly as a GitHub Actions job does. The API URL is
// unroutable; the watchdog never reaches it before the command exits.
const watchdogEnv = {
  ...process.env,
  GITHUB_ACTIONS: "true",
  GITHUB_RUN_ID: "1",
  GITHUB_REPOSITORY: "example/repo",
  GITHUB_API_URL: "http://127.0.0.1:9",
  GH_TOKEN: "test-token",
  CI_CANCEL_WATCHDOG: "1",
  CI_CANCEL_POLL_SECONDS: "5",
};

test("captured output returns when the command exits, not after a watchdog poll", () => {
  const started = Date.now();
  const { stdout, stderr, status } = spawnSync("bash", [script, "echo", "done"], {
    env: watchdogEnv,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });
  const elapsed = Date.now() - started;

  assert.equal(status, 0);

  assert.equal(stdout.trim(), "done");
  assert.ok(!stderr.includes("Terminated"), `watchdog shutdown leaked noise: ${stderr}`);
  // The orphaned `sleep` of a killed watchdog used to hold stdout open until
  // the poll interval elapsed, so every captured call cost up to 20 s.
  assert.ok(elapsed < 3000, `wrapper held output open for ${elapsed} ms`);
});

test("the wrapped command's exit status is preserved with the watchdog armed", () => {
  assert.throws(
    () =>
      execFileSync("bash", [script, "bash", "-c", "exit 7"], {
        env: watchdogEnv,
        stdio: ["ignore", "pipe", "pipe"],
      }),
    (error) => error.status === 7,
  );
});
