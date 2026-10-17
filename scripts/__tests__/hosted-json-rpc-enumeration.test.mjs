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
    const command = block[1]
      .replace(/^          /gm, "")
      .replaceAll("/root/", `${root}/`);
    for (const directory of [
      "bin",
      "scripts/ci",
      "tests/raw_coverage",
      "tests/in_process",
      "modules",
    ])
      mkdirSync(join(root, directory), { recursive: true });
    writeFileSync(join(root, "tests/json_rpc_e2e.rs"), "fixture");
    writeFileSync(join(root, "tests/in_process_all.rs"), "fixture");
    writeFileSync(join(root, "tests/in_process/domain_modules_e2e.rs"), "fixture");
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
if [[ "$*" == *"test -p openhuman --lib"* ]]; then
  [[ -n "\${XDG_CACHE_HOME:-}" && -n "\${XDG_DATA_HOME:-}" && "$XDG_CACHE_HOME" != "$XDG_DATA_HOME" && -d "$XDG_CACHE_HOME" && -d "$XDG_DATA_HOME" ]] || {
    echo "core tests need separate private XDG cache and data directories" >&2
    exit 13
  }
  if [[ "$*" == *"--test-threads=1"* ]]; then
    [[ "\${OPENHUMAN_MODULE_PATH:-}" == "$MODULE_FIXTURE_PATH" ]] || {
      echo "module-backed core tests need the staged module path" >&2
      exit 15
    }
    if [[ "$*" == *"cron::bus_host::tests::"* || "$*" == *"cron::documents_port::tests::"* ]]; then
      echo "ISOLATED-CRON-MODULE-TEST:$*"
    elif [[ "$*" == *"inference::tokenjuice::"* || "$*" == *"tinyjuice_detects_and_converts_html"* ]]; then
      echo "ISOLATED-MODULE-TEST:$*"
    fi
    echo "test result: ok. 1 passed; 0 failed; 0 ignored"
  else
    [[ -z "\${OPENHUMAN_MODULE_PATH:-}" ]] || {
      echo "parallel core unit tests must not load process-global modules" >&2
      exit 14
    }
    [[ "$*" == *"--skip cron::bus_host::tests::"* && "$*" == *"--skip cron::documents_port::tests::"* ]] || {
      echo "parallel core unit tests must skip module-backed cron tests" >&2
      exit 16
    }
    echo CORE-TESTS-WITHOUT-MODULE-PATH
  fi
  echo PRIVATE-XDG-CACHE
fi
if [[ "$*" == *"--test in_process_all"* ]]; then
  if [[ "$*" == *"--skip domain_modules_e2e::cron_update_applies_a_partial_patch_without_clobbering_unset_fields"* &&
        "$*" == *"--skip domain_modules_e2e::mcp_audit_list_dispatches_internally_and_stays_out_of_the_schema_dump"* &&
        "$*" == *"--skip domain_modules_e2e::mcp_clients_read_paths_validate_before_reaching_outward"* &&
        "$*" == *"--skip domain_modules_e2e::mcp_tool_description_scan_flags_injection_and_spares_ordinary_prose"* ]]; then
    [[ "$*" == *"--test-threads=1"* ]] || exit 17
    echo IN_PROCESS_DOMAIN_GROUP_EXCLUDES_MODULE_CASES
  elif [[ "$*" == *"--exact"* &&
          ( "$*" == *"domain_modules_e2e::cron_update_applies_a_partial_patch_without_clobbering_unset_fields"* ||
            "$*" == *"domain_modules_e2e::mcp_audit_list_dispatches_internally_and_stays_out_of_the_schema_dump"* ||
            "$*" == *"domain_modules_e2e::mcp_clients_read_paths_validate_before_reaching_outward"* ||
            "$*" == *"domain_modules_e2e::mcp_tool_description_scan_flags_injection_and_spares_ordinary_prose"* ) ]]; then
    [[ "$*" == *"--test-threads=1"* ]] || exit 18
    echo "ISOLATED-IN-PROCESS-E2E-TEST:$*"
  else
    exit 19
  fi
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
        MODULE_FIXTURE_PATH: join(root, "modules"),
        OPENHUMAN_MODULE_PATH: join(root, "modules"),
      },
    });
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

test("hosted JSON-RPC tests are listed and executed with product features", () => {
  const result = runCoreStep("normal");
  assert.equal(result.status, 0, result.stdout + result.stderr);
  assert.match(result.stdout, /CORE-TESTS-WITHOUT-MODULE-PATH/);
  assert.match(result.stdout, /ISOLATED-MODULE-TEST/);
  assert.equal(
    (result.stdout.match(/ISOLATED-MODULE-TEST:/g) ?? []).length,
    4,
    result.stdout,
  );
  assert.equal(
    (result.stdout.match(/ISOLATED-CRON-MODULE-TEST:/g) ?? []).length,
    13,
    result.stdout,
  );
  assert.match(result.stdout, /IN_PROCESS_DOMAIN_GROUP_EXCLUDES_MODULE_CASES/);
  assert.equal(
    (result.stdout.match(/ISOLATED-IN-PROCESS-E2E-TEST:/g) ?? []).length,
    4,
    result.stdout,
  );
  assert.match(result.stdout, /PRIVATE-XDG-CACHE/);
  assert.match(result.stdout, /EXECUTED-JSON-RPC/);
});

for (const mode of ["broken", "empty"]) {
  test(`hosted JSON-RPC ${mode} enumeration cannot report a pass`, () => {
    const result = runCoreStep(mode);
    assert.notEqual(result.status, 0, result.stdout + result.stderr);
    assert.doesNotMatch(result.stdout, /EXECUTED-JSON-RPC/);
  });
}
