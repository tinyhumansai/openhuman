import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const stage = fileURLToPath(
  new URL("../ci/stage-connector-test-module.mjs", import.meta.url),
);

test("locally built connector fixtures carry the exact artifact digest for confidential calls", () => {
  const root = mkdtempSync(join(tmpdir(), "connector-fixture-"));
  try {
    const source = join(root, "libtinyconnectors.so");
    const destination = join(root, "fixture");
    for (const bytes of ["first pinned build", "replacement pinned build"]) {
      writeFileSync(source, bytes);
      const result = spawnSync(process.execPath, [stage, source, destination], {
        encoding: "utf8",
      });
      assert.equal(result.status, 0, result.stdout + result.stderr);
      const fixture = result.stdout.trim();
      assert.equal(readFileSync(fixture, "utf8"), bytes);
      const digest = createHash("sha256").update(bytes).digest("hex");
      assert.equal(
        readFileSync(join(destination, "modules.toml"), "utf8"),
        `"libtinyconnectors.so" = "${digest}"\n`,
      );
    }
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("native E2E installation attests the connector module before launching the app", () => {
  const root = mkdtempSync(join(tmpdir(), "native-connector-fixture-"));
  try {
    const workflow = readFileSync(
      new URL("../../.github/workflows/e2e-reusable.yml", import.meta.url),
      "utf8",
    );
    const block = workflow.match(
      /- name: Install native E2E test modules\n        run: \|\n([\s\S]*?)(?=\n      - name:)/,
    );
    assert.ok(block, "native module installation step exists");
    const command = block[1]
      .replace(/^          /gm, "")
      .replace("/opt/openhuman-test-modules", join(root, "modules"));
    const bytes = "pinned native connector module";
    const bin = join(root, "bin");
    const cache = join(root, "cache");
    mkdirSync(bin);
    mkdirSync(join(cache, "blobs"), { recursive: true });
    const rustcVersion = "fixture rustc\n";
    const rustcDigest = createHash("sha256")
      .update(rustcVersion)
      .digest("hex")
      .slice(0, 12);
    writeFileSync(
      join(cache, "blobs", `tinyconnectors-fixture-${rustcDigest}.so`),
      bytes,
    );
    const assets = [];
    for (const id of [
      "tinyjuice",
      "tinysearch",
      "tinydocs",
      "tinywallet",
      "tinychannels",
      "tinymcp",
    ]) {
      const archive = `${id}.tar.gz`;
      const contents = join(root, `archive-${id}`);
      mkdirSync(contents);
      writeFileSync(join(contents, "module-fixture.so"), `released ${id}`);
      const packed = spawnSync("tar", [
        "-czf",
        join(root, archive),
        "-C",
        contents,
        ".",
      ]);
      assert.equal(packed.status, 0, packed.stderr?.toString());
      const sha = createHash("sha256")
        .update(readFileSync(join(root, archive)))
        .digest("hex");
      writeFileSync(
        join(cache, "blobs", `${sha}-${archive}`),
        readFileSync(join(root, archive)),
      );
      assets.push(
        [id, "https://fixture.invalid/unused", archive, sha].join("\t"),
      );
    }
    writeFileSync(join(root, "assets.tsv"), assets.join("\n") + "\n");
    writeFileSync(
      join(bin, "node"),
      `#!/bin/sh\ncase "$1" in *test-module-assets.mjs) cat assets.tsv;; *) exec '${process.execPath}' "$@";; esac\n`,
      { mode: 0o755 },
    );
    writeFileSync(
      join(bin, "git"),
      '#!/bin/sh\ncase "$*" in *rev-parse*) echo fixture;; esac\n',
      { mode: 0o755 },
    );
    writeFileSync(join(bin, "rustc"), `#!/bin/sh\nprintf '${rustcVersion}'\n`, {
      mode: 0o755,
    });
    for (const command of ["curl", "cargo"]) {
      writeFileSync(
        join(bin, command),
        "#!/bin/sh\necho unexpected build or download >&2\nexit 99\n",
        { mode: 0o755 },
      );
    }
    mkdirSync(join(root, "scripts/ci/self-hosted"), { recursive: true });
    writeFileSync(
      join(root, "scripts/ci/stage-connector-test-module.mjs"),
      readFileSync(stage),
    );
    writeFileSync(
      join(root, "scripts/ci/self-hosted/install-test-modules.sh"),
      readFileSync(
        new URL("../ci/self-hosted/install-test-modules.sh", import.meta.url),
      ),
    );
    const envFile = join(root, "env");
    const result = spawnSync("bash", ["-c", command], {
      cwd: root,
      encoding: "utf8",
      env: {
        ...process.env,
        GITHUB_RUN_ID: "fixture",
        GITHUB_RUN_ATTEMPT: "1",
        GITHUB_ENV: envFile,
        PATH: `${bin}:${process.env.PATH}`,
        CI_CACHE_DIR: cache,
        OPENHUMAN_MODULE_PATH: "",
      },
    });
    assert.equal(result.status, 0, result.stdout + result.stderr);
    const destination = join(root, "modules/fixture-1/tinyconnectors");
    assert.equal(
      readFileSync(join(destination, "libtinyconnectors.so"), "utf8"),
      bytes,
    );
    assert.equal(
      readFileSync(join(destination, "modules.toml"), "utf8"),
      `"libtinyconnectors.so" = "${createHash("sha256").update(bytes).digest("hex")}"\n`,
    );
    assert.equal(
      readFileSync(envFile, "utf8").trim(),
      [
        `TINYJUICE_TEST_MODULE=${join(root, "modules/fixture-1/tinyjuice/libtinyjuice_module.so")}`,
        `TINYSEARCH_TEST_MODULE=${join(root, "modules/fixture-1/tinysearch/libtinysearch.so")}`,
        `OPENHUMAN_MODULE_PATH=${["tinydocs", "tinywallet", "tinychannels", "tinymcp"].map((id) => join(root, "modules/fixture-1", id)).join(":")}`,
        `TINYCONNECTORS_TEST_MODULE=${destination}/libtinyconnectors.so`,
      ].join("\n"),
    );
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
