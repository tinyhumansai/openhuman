import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  mkdtempSync,
  mkdirSync,
  writeFileSync,
  existsSync,
  rmSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import {
  readRegistrySource,
  resolveTestModuleAssets,
} from "../ci/self-hosted/test-module-assets.mjs";

test("native document regressions receive a checksum-pinned module fixture", () => {
  const assets = resolveTestModuleAssets(
    readRegistrySource(),
    "ubuntu-22.04-x86_64",
  );
  const documents = assets.find((asset) => asset.id === "tinydocs");
  assert.ok(
    documents,
    "the full Rust suite must include its native document fixture",
  );
  assert.match(documents.url, /\/releases\/download\/v\d+\.\d+\.\d+\//);
  assert.match(documents.sha256, /^[a-f0-9]{64}$/);
});

const installer = fileURLToPath(
  new URL("../ci/self-hosted/install-test-modules.sh", import.meta.url),
);

for (const [description, resolverExit] of [
  ["failed", 7],
  ["empty", 0],
]) {
  test(`${description} module asset resolution refuses to publish fixture paths`, () => {
    const root = mkdtempSync(join(tmpdir(), "module-install-"));
    try {
      const bin = join(root, "bin");
      mkdirSync(bin);
      writeFileSync(join(bin, "node"), `#!/bin/sh\nexit ${resolverExit}\n`, {
        mode: 0o755,
      });
      // No builds or downloads are needed to verify failure admission.
      for (const command of ["git", "cargo", "rustc", "curl"]) {
        writeFileSync(
          join(bin, command),
          `#!/bin/sh\ntouch '${root}/attempted'\nexit 0\n`,
          { mode: 0o755 },
        );
      }
      const envFile = join(root, "modules.env");
      const result = spawnSync(
        "bash",
        [installer, join(root, "modules"), envFile],
        {
          encoding: "utf8",
          env: {
            ...process.env,
            PATH: `${bin}:${process.env.PATH}`,
            CI_CACHE_DIR: "",
          },
        },
      );
      assert.notEqual(result.status, 0, result.stdout + result.stderr);
      assert.equal(
        existsSync(join(root, "attempted")),
        false,
        "resolve pins before building or downloading",
      );
      assert.equal(
        existsSync(envFile),
        false,
        "unresolved modules must not be exported",
      );
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });
}

for (const id of ["tinywallet", "tinychannels", "tinymcp"]) {
  test(`native ${id} regressions receive their pinned release fixture`, () => {
    const assets = resolveTestModuleAssets(
      readRegistrySource(),
      "ubuntu-22.04-x86_64",
    );
    const artifact = assets.find((asset) => asset.id === id);
    assert.ok(
      artifact,
      `${id} must not fall back to an untrusted runner cache`,
    );
    assert.match(artifact.sha256, /^[a-f0-9]{64}$/);
    assert.match(artifact.url, /\/releases\/download\/v/);
  });
}
