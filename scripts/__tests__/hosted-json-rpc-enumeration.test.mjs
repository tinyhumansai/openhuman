import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  mkdtempSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

function runCoreStep(mode) {
  const root = mkdtempSync(join(tmpdir(), "hosted-json-rpc-"));
  try {
    const workflow = readFileSync(
      new URL("../../.github/workflows/test-reusable.yml", import.meta.url),
      "utf8",
    );
    const block = workflow.match(
      /- name: Test core crate \(openhuman\)[\s\S]*?        run: \|\n([\s\S]*?)(?=\n  rust-core-tests-windows:)/,
    );
    assert.ok(block, "hosted core step exists");
    const command = block[1].replace(/^          /gm, "");
    for (const directory of [
      "bin",
      "scripts/ci",
      "tests/raw_coverage",
      "tests/in_process",
    ])
      mkdirSync(join(root, directory), { recursive: true });
    writeFileSync(join(root, "tests/json_rpc_e2e.rs"), "fixture");
    writeFileSync(join(root, "scripts/ci/product-features.sh"), "echo voice\n");
    writeFileSync(join(root, "scripts/ci-cancel-aware.sh"), 'exec "$@"\n');
    writeFileSync(join(root, "scripts/run-embed-examples.mjs"), "");
    writeFileSync(
      join(root, "bin/cargo"),
      `#!/bin/bash
if [[ "$*" == *"--test json_rpc_e2e"* ]]; then
  [[ "$*" == *"--features voice,bin-tools"* ]] || exit 0
  if [[ "$*" == *"--list"* ]]; then
    case "$FIXTURE_MODE" in
      broken) exit 7;;
      empty) exit 0;;
      *) echo 'json_rpc_fixture: test'; exit 0;;
    esac
  fi
  [[ "$*" == *"json_rpc_fixture"* ]] || exit 9
  echo EXECUTED-JSON-RPC
fi
`,
      { mode: 0o755 },
    );
    return spawnSync("bash", ["-c", command], {
      cwd: root,
      encoding: "utf8",
      env: {
        ...process.env,
        PATH: `${join(root, "bin")}:${process.env.PATH}`,
        FIXTURE_MODE: mode,
      },
    });
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

test("hosted JSON-RPC tests are listed and executed with product features", () => {
  const result = runCoreStep("normal");
  assert.equal(result.status, 0, result.stdout + result.stderr);
  assert.match(result.stdout, /EXECUTED-JSON-RPC/);
});

for (const mode of ["broken", "empty"]) {
  test(`hosted JSON-RPC ${mode} enumeration cannot report a pass`, () => {
    const result = runCoreStep(mode);
    assert.notEqual(result.status, 0, result.stdout + result.stderr);
    assert.doesNotMatch(result.stdout, /EXECUTED-JSON-RPC/);
  });
}
