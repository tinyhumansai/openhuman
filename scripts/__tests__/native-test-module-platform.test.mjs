import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import test from "node:test";

const helper = fileURLToPath(
  new URL("../ci/native-test-module-platform.sh", import.meta.url),
);

function resolve(host, macosMajor) {
  return spawnSync("bash", [helper, host, String(macosMajor)], {
    encoding: "utf8",
  });
}

test("native Mac module archives follow the Rust target under Rosetta", () => {
  assert.equal(
    resolve("aarch64-apple-darwin", 26).stdout,
    "dylib\tmacos-26-arm64\n",
  );
  assert.equal(
    resolve("aarch64-apple-darwin", 15).stdout,
    "dylib\tmacos-15-arm64\n",
  );
  assert.equal(
    resolve("x86_64-apple-darwin", 26).stdout,
    "dylib\tmacos-26-x86_64\n",
  );
});

test("native Linux module archives match the Rust target", () => {
  assert.equal(
    resolve("x86_64-unknown-linux-gnu").stdout,
    "so\tubuntu-22.04-x86_64\n",
  );
  assert.equal(
    resolve("aarch64-unknown-linux-gnu").stdout,
    "so\tubuntu-22.04-arm64\n",
  );
});

test("unsupported Rust targets fail closed", () => {
  const result = resolve("x86_64-pc-windows-msvc");
  assert.equal(result.status, 1);
  assert.match(result.stderr, /Unsupported native-module test target/);
});
