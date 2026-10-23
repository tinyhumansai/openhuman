import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { chmodSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const fixture = fileURLToPath(
  new URL("../ci/security-browser-egress-fixture.sh", import.meta.url),
);

test("native browser egress fixture runs security and opt-in Chromium tests", () => {
  const dir = mkdtempSync(join(tmpdir(), "security-browser-egress-fixture-"));
  const log = join(dir, "invocations.txt");
  const bash = join(dir, "bash");
  writeFileSync(
    bash,
    `#!/bin/sh\nprintf 'ARGS %s\\nDNS %s\\n' "$*" "\${TINYSECURITY_TEST_DNS_SEQUENCE:-}" >> "${log}"\n`,
  );
  chmodSync(bash, 0o755);

  const result = spawnSync("/bin/bash", [fixture], {
    encoding: "utf8",
    env: {
      ...process.env,
      PATH: `${dir}:${process.env.PATH}`,
      OPENHUMAN_TEST_SECURITY_MODULE: fixture,
    },
  });

  assert.equal(result.status, 0, result.stderr);
  const calls = readFileSync(log, "utf8");
  assert.match(calls, /cargo build -p tinysecurity-module/);
  assert.match(calls, /cargo build -p tinysecurity-module --features native-security-tests/);
  assert.match(calls, /cargo build -p tinycomputer/);
  assert.match(calls, /cargo test -p openhuman --lib/);
  assert.match(calls, /--features[^\n]*native-security-tests/);
  const invocations = calls.split("\n");
  const securityTest = invocations.find((line) =>
    line.includes(
      "modules::browser_egress_host::tests::browser_egress_callbacks_use_tinysecurity_one_use_grants_and_leases",
    ),
  );
  assert.ok(securityTest, "TinySecurity callback test is selected");
  assert.match(securityTest, /-- --exact$/);
  assert.doesNotMatch(securityTest, /--ignored/);
  const browserTest = invocations.find((line) =>
    line.includes(
      "modules::browser_egress_host::tests::loaded_tinycomputer_uses_tinysecurity_to_block_loopback_dials",
    ),
  );
  assert.ok(browserTest, "real-module Chromium test is selected");
  assert.match(browserTest, /-- --ignored --exact$/);
  const rebindingTest = invocations.find((line) =>
    line.includes(
      "modules::browser_egress_host::tests::loaded_tinycomputer_rejects_public_to_loopback_dns_rebinding",
    ),
  );
  assert.ok(rebindingTest, "controlled DNS rebinding test is selected");
  assert.match(rebindingTest, /-- --ignored --exact$/);
  assert.match(
    invocations.join("\n"),
    /DNS \{"rebind\.test":\["8\.8\.8\.8","127\.0\.0\.1"\],"metadata\.test":\["8\.8\.8\.8","169\.254\.169\.254","169\.254\.169\.254"\]\}/,
  );
});
