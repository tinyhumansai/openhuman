import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, mkdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const helper = fileURLToPath(
  new URL("../ci/trust-checkout.sh", import.meta.url),
);

test(
  "container checkout trust covers its submodules and excludes unrelated repositories",
  { skip: process.platform === "darwin" && "Git ownership checks are unavailable on macOS" },
  () => {
  const root = mkdtempSync(join(tmpdir(), "checkout-trust-"));
  const env = {
    ...process.env,
    GIT_CONFIG_GLOBAL: join(root, "gitconfig"),
    GIT_CONFIG_NOSYSTEM: "1",
  };
  const git = (directory, args, extra = {}) =>
    spawnSync("git", ["-C", directory, ...args], {
      encoding: "utf8",
      env: { ...env, ...extra },
    });
  const checked = (directory, args) => {
    const result = git(directory, args);
    assert.equal(result.status, 0, result.stdout + result.stderr);
  };
  try {
    for (const name of ["checkout", "dependency", "nested", "unrelated"]) {
      const directory = join(root, name);
      mkdirSync(directory);
      checked(directory, ["init", "--quiet"]);
      checked(directory, [
        "-c",
        "user.name=Fixture",
        "-c",
        "user.email=fixture@example.invalid",
        "-c",
        "core.hooksPath=/dev/null",
        "commit",
        "--allow-empty",
        "--quiet",
        "-m",
        "fixture",
      ]);
    }
    checked(join(root, "dependency"), [
      "-c",
      "protocol.file.allow=always",
      "submodule",
      "add",
      "--quiet",
      join(root, "nested"),
      "vendor/nested",
    ]);
    checked(join(root, "dependency"), [
      "-c",
      "user.name=Fixture",
      "-c",
      "user.email=fixture@example.invalid",
      "-c",
      "core.hooksPath=/dev/null",
      "commit",
      "--quiet",
      "-am",
      "nested fixture",
    ]);
    const checkout = join(root, "checkout");
    checked(checkout, [
      "-c",
      "protocol.file.allow=always",
      "submodule",
      "add",
      "--quiet",
      join(root, "dependency"),
      "vendor/dependency",
    ]);
    checked(checkout, [
      "-c",
      "protocol.file.allow=always",
      "submodule",
      "update",
      "--init",
      "--recursive",
    ]);
    const foreign = { GIT_TEST_ASSUME_DIFFERENT_OWNER: "1" };
    assert.notEqual(
      git(checkout, ["status", "--short"], foreign).status,
      0,
      "reproduce the container ownership failure",
    );
    const result = spawnSync("bash", [helper, checkout], {
      encoding: "utf8",
      env: { ...env, ...foreign },
    });
    assert.equal(result.status, 0, result.stdout + result.stderr);
    assert.equal(git(checkout, ["status", "--short"], foreign).status, 0);
    assert.equal(
      git(join(checkout, "vendor/dependency"), ["status", "--short"], foreign)
        .status,
      0,
    );
    assert.equal(
      git(
        join(checkout, "vendor/dependency/vendor/nested"),
        ["status", "--short"],
        foreign,
      ).status,
      0,
    );
    assert.notEqual(
      git(join(root, "unrelated"), ["status", "--short"], foreign).status,
      0,
    );
    assert.doesNotMatch(
      git(checkout, ["config", "--global", "--get-all", "safe.directory"])
        .stdout,
      /^\*$/m,
    );
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
  },
);
