import assert from "node:assert/strict";
import { test } from "node:test";
import { checkManifest } from "../ci/check-security-module-dependencies.mjs";

test("allows only the security bus", () => {
  assert.deepEqual(
    checkManifest('[dependencies]\ntinysecurity-bus = "0.17"', "crates/openhuman-core/Cargo.toml"),
    [],
  );
});

test("rejects every other TinySecurity crate from core and host manifests", () => {
  for (const [filename, packageName] of [
    ["crates/openhuman-core/Cargo.toml", "tinysecurity-approval"],
    ["crates/openhuman-core/Cargo.toml", "tinysecurity-crypto"],
    ["crates/openhuman-rpc/Cargo.toml", "tinysecurity-crypto"],
  ]) {
    assert.equal(
      checkManifest(`[dependencies]\n${packageName} = "0.17"`, filename).length,
      1,
      filename,
    );
  }
});

test("rejects the engine crate from target-specific and patch tables", () => {
  assert.equal(
    checkManifest(
      '[target."cfg(unix)".dependencies]\ncrypto = { package = "tinysecurity-crypto", version = "0.17" }\n[patch.crates-io]\ntinysecurity = "0.17"',
      "crates/openhuman-app/Cargo.toml",
    ).length,
    2,
  );
});
