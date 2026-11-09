import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
  existsSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

const source = readFileSync(
  new URL("../test-rust-with-mock.sh", import.meta.url),
  "utf8",
);
const start = source.indexOf("run_json_rpc_e2e() {");
const end = source.indexOf("\n}\n", start);
assert.ok(start >= 0 && end > start);
const body = source.slice(start, end + 3);

function extractFunction(name) {
  const functionStart = source.indexOf(`${name}() {`);
  const functionEnd = source.indexOf("\n}\n", functionStart);
  assert.ok(functionStart >= 0 && functionEnd > functionStart, `${name} exists`);
  return source.slice(functionStart, functionEnd + 3);
}

const fullSuite = extractFunction("run_full_suite");
const dispatchStart = source.indexOf('if [ "$#" -eq 0 ]; then\n  run_full_suite');
assert.ok(dispatchStart >= 0, "explicit target dispatch block exists");
const dispatch = source.slice(dispatchStart);

const JSON_RPC_TARGETS = [
  "json_rpc_threads_e2e",
  "json_rpc_wallet_e2e",
  "json_rpc_flows_e2e",
  "json_rpc_flows_builder_chat_e2e",
  "json_rpc_mcp_e2e",
  "json_rpc_inference_e2e",
  "json_rpc_auth_security_e2e",
  "json_rpc_config_e2e",
  "json_rpc_agent_team_e2e",
  "json_rpc_web_chat_e2e",
  "json_rpc_misc_e2e",
];

for (const [description, listing, code] of [
  ["failed", "", 101],
  ["partial", "scenario_a: test\n", 101],
  ["empty", "0 tests, 0 benchmarks\n", 0],
  ["complete", "scenario_a: test\n", 0],
]) {
  test(`the full product runner checks ${description} JSON-RPC enumeration`, () => {
    const root = mkdtempSync(join(tmpdir(), "product-enumeration-"));
    try {
      const cargo = join(root, "cargo");
      writeFileSync(
        cargo,
        '#!/bin/bash\nif [[ " $* " == *" --list "* ]]; then\n printf "%s" "$TEST_LISTING"\n exit "$TEST_LIST_CODE"\nfi\nprintf "%s" "$*" > "$TEST_RAN"\n',
        { mode: 0o755 },
      );
      const ran = join(root, "ran");
      const result = spawnSync(
        "bash",
        [
          "-c",
          'set -euo pipefail\ncargo_test() { "$TEST_CARGO" "$@"; }\n' +
            body +
            "\nrun_json_rpc_e2e\n",
        ],
        {
          encoding: "utf8",
          env: {
            ...process.env,
            TEST_CARGO: cargo,
            TEST_LISTING: listing,
            TEST_LIST_CODE: String(code),
            TEST_RAN: ran,
          },
        },
      );
      if (description === "complete") {
        assert.equal(result.status, 0, result.stdout + result.stderr);
        assert.match(
          readFileSync(ran, "utf8"),
          /--test json_rpc_misc_e2e scenario_a -- --exact --test-threads=1/,
        );
      } else {
        assert.notEqual(result.status, 0, result.stdout + result.stderr);
        assert.equal(
          existsSync(ran),
          false,
          "an incomplete list must run no cases",
        );
      }
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });
}

test("the mock runner rejects a filter that matches no JSON-RPC test", () => {
  const root = mkdtempSync(join(tmpdir(), "product-enumeration-filter-"));
  try {
    const cargo = join(root, "cargo");
    writeFileSync(
      cargo,
      '#!/bin/bash\nif [[ " $* " == *" --list "* ]]; then echo "scenario_a: test"; exit 0; fi\nprintf "%s\\n" "$*" >> "$TEST_RAN"\n',
      { mode: 0o755 },
    );
    const ran = join(root, "ran");
    const result = spawnSync(
      "bash",
      [
        "-c",
        'set -euo pipefail\ncargo_test() { "$TEST_CARGO" "$@"; }\n' +
          body +
          '\nJSON_RPC_E2E_FILTER=definitely_no_matching_case run_json_rpc_e2e\n',
      ],
      {
        encoding: "utf8",
        env: {
          ...process.env,
          TEST_CARGO: cargo,
          TEST_RAN: ran,
        },
      },
    );
    assert.notEqual(
      result.status,
      0,
      "an unmatched filter must fail instead of silently skipping the suite",
    );
    assert.equal(existsSync(ran), false, "no test case should run for an unmatched filter");
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("a filter may skip individual JSON-RPC targets when another target matches", () => {
  const root = mkdtempSync(join(tmpdir(), "product-enumeration-filter-hit-"));
  try {
    const cargo = join(root, "cargo");
    writeFileSync(
      cargo,
      `#!/bin/bash
if [[ " $* " == *" --list "* ]]; then
  if [[ " $* " == *" --test json_rpc_threads_e2e "* ]]; then echo "matching_case: test"; else echo "other_case: test"; fi
  exit 0
fi
printf "%s\\n" "$*" >> "$TEST_RAN"
`,
      { mode: 0o755 },
    );
    const ran = join(root, "ran");
    const result = spawnSync(
      "bash",
      [
        "-c",
        'set -euo pipefail\ncargo_test() { "$TEST_CARGO" "$@"; }\n' +
          body +
          '\nJSON_RPC_E2E_FILTER=matching_case run_json_rpc_e2e\n',
      ],
      {
        encoding: "utf8",
        env: { ...process.env, TEST_CARGO: cargo, TEST_RAN: ran },
      },
    );
    assert.equal(result.status, 0, result.stdout + result.stderr);
    assert.match(readFileSync(ran, "utf8"), /--test json_rpc_threads_e2e matching_case/);
    assert.equal(readFileSync(ran, "utf8").trim().split("\n").length, 1);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("the full product runner enumerates and runs each JSON-RPC target once", () => {
  const root = mkdtempSync(join(tmpdir(), "product-suite-routing-"));
  try {
    const log = join(root, "cargo.log");
    const script = [
      "set -euo pipefail",
      `RUN_LOG='${log}'`,
      "connectors_module_path=fixture hosts_dir=fixture",
      'cargo_test() {\n  printf "CARGO\\t%s\\n" "$*" >> "$RUN_LOG"\n  local previous= target= arg\n  for arg in "$@"; do\n    if [ "$previous" = "--test" ]; then target="$arg"; fi\n    previous="$arg"\n  done\n  if [[ " $* " == *" -- --list "* ]]; then printf "%s_case: test\\n" "$target"; fi\n}',
      'integration_test_targets() { printf "%s\\n" ' + JSON_RPC_TARGETS.join(" ") + "; }",
      "run_build_only_reaper_test() { :; }",
      "run_tinychannels_linked_fixture_tests() { :; }",
      "run_raw_coverage_modules() { :; }",
      "run_in_process_modules() { :; }",
      body,
      fullSuite,
      "run_full_suite",
    ].join("\n");
    const result = spawnSync("bash", ["-c", script], { encoding: "utf8" });
    assert.equal(result.status, 0, result.stdout + result.stderr);
    const calls = readFileSync(log, "utf8").split("\n").filter(Boolean);
    for (const target of JSON_RPC_TARGETS) {
      assert.equal(
        calls.filter((line) => line.includes(`--test ${target} -- --list`)).length,
        1,
        `${target} should be enumerated once`,
      );
      assert.equal(
        calls.filter((line) => line.includes(`--test ${target} ${target}_case --`)).length,
        1,
        `${target} case should run once`,
      );
    }
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("an explicit JSON-RPC namespace target uses isolated enumeration", () => {
  const root = mkdtempSync(join(tmpdir(), "product-target-routing-"));
  try {
    const log = join(root, "dispatch.log");
    const script = [
      "set -euo pipefail",
      `RUN_LOG='${log}'`,
      'cargo_test() { printf "CARGO\\t%s\\n" "$*" >> "$RUN_LOG"; }',
      'run_json_rpc_e2e() { printf "JSON_RPC\\t[%s]\\n" "$*" >> "$RUN_LOG"; }',
      "run_full_suite() { :; }",
      "run_raw_coverage_modules() { :; }",
      "run_in_process_modules() { :; }",
      dispatch,
    ].join("\n");
    const result = spawnSync("bash", ["-c", script, "bash", "--test", "json_rpc_threads_e2e"], {
      encoding: "utf8",
    });
    assert.equal(result.status, 0, result.stdout + result.stderr);
    assert.deepEqual(readFileSync(log, "utf8").trim().split("\n"), [
      "JSON_RPC\t[json_rpc_threads_e2e]",
    ]);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("the legacy JSON-RPC aggregate alias still selects every namespace", () => {
  const root = mkdtempSync(join(tmpdir(), "product-alias-routing-"));
  try {
    const log = join(root, "dispatch.log");
    const script = [
      "set -euo pipefail",
      `RUN_LOG='${log}'`,
      'cargo_test() { printf "CARGO\\t%s\\n" "$*" >> "$RUN_LOG"; }',
      'run_json_rpc_e2e() { printf "JSON_RPC\\t[%s]\\n" "$*" >> "$RUN_LOG"; }',
      "run_full_suite() { :; }",
      "run_raw_coverage_modules() { :; }",
      "run_in_process_modules() { :; }",
      dispatch,
    ].join("\n");
    const result = spawnSync("bash", ["-c", script, "bash", "--test", "json_rpc_e2e"], {
      encoding: "utf8",
    });
    assert.equal(result.status, 0, result.stdout + result.stderr);
    assert.deepEqual(readFileSync(log, "utf8").trim().split("\n"), ["JSON_RPC\t[]"]);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("released TinyHosts contract callers match the declared libtest module", () => {
  const moduleSource = readFileSync(
    new URL("../../crates/openhuman-core/src/hosting/mod.rs", import.meta.url),
    "utf8",
  );
  const moduleName = moduleSource.match(
    /#\[path = "hosting_tests\.rs"\]\s*mod (\w+);/,
  )?.[1];
  assert.ok(moduleName, "hosting contract test module is declared");
  const filter = `hosting::${moduleName}::released_artifact_serves_providers_and_confidential_local_preparation`;
  for (const caller of [
    "../test-rust-with-mock.sh",
    "../../.github/workflows/test-reusable.yml",
  ]) {
    assert.ok(
      readFileSync(new URL(caller, import.meta.url), "utf8").includes(filter),
      `${caller} must invoke the registered contract test`,
    );
  }
});
