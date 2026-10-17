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
          /--test json_rpc_e2e scenario_a -- --exact --test-threads=1/,
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
