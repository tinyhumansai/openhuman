import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  mkdtempSync,
  mkdirSync,
  readFileSync,
  writeFileSync,
  existsSync,
  rmSync,
  statSync,
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
const localRustRunner = fileURLToPath(
  new URL("../test-rust-with-mock.sh", import.meta.url),
);
const rustE2eRunner = fileURLToPath(
  new URL("../test-rust-e2e.sh", import.meta.url),
);

test("Linux container workflows stage TinyBus fixtures under the root-owned container home", () => {
  for (const workflow of [
    "../../.github/workflows/test-reusable.yml",
    "../../.github/workflows/ci-full.yml",
    "../../.github/workflows/e2e-reusable.yml",
    "../../.github/workflows/e2e-playwright.yml",
  ]) {
    const source = readFileSync(new URL(workflow, import.meta.url), "utf8");
    assert.match(
      source,
      /\/root\/openhuman-test-modules-\$\{GITHUB_RUN_ID\}-\$\{GITHUB_RUN_ATTEMPT\}/,
      workflow,
    );
    assert.doesNotMatch(source, /\/opt\/openhuman-test-modules/, workflow);
    assert.doesNotMatch(
      source,
      /mktemp -d \/tmp\/openhuman-(?:core|e2e|rust-e2e)/,
      workflow,
    );
  }

  const ciFull = readFileSync(
    new URL("../../.github/workflows/ci-full.yml", import.meta.url),
    "utf8",
  );
  assert.match(
    ciFull,
    /OPENHUMAN_XDG_ROOT: \/root\/openhuman-rust-e2e-xdg-\$\{\{ github\.run_id \}\}/,
  );
  const rustE2eRunnerSource = readFileSync(rustE2eRunner, "utf8");
  assert.match(
    rustE2eRunnerSource,
    /XDG_ROOT="\$\{OPENHUMAN_XDG_ROOT:-\$\{TMPDIR:-\/tmp\}\}"/,
  );

  const lanes = readFileSync(
    new URL("../ci/self-hosted/lanes-plan.mjs", import.meta.url),
    "utf8",
  );
  assert.match(
    lanes,
    /\/tmp\/openhuman-test-modules-\$\{GITHUB_RUN_ID\}-\$\{GITHUB_RUN_ATTEMPT\}/,
  );
  assert.doesNotMatch(lanes, /\/opt\/openhuman-test-modules/);
});

test("locally built native Rust test modules come from build.mjs, not ad hoc cargo builds", () => {
  const source = readFileSync(localRustRunner, "utf8");
  assert.match(source, /modules\/stage-local-modules\.sh/);
  assert.match(source, /stage_source_modules "\$\{local_source_ids\[@\]\}"/);
  assert.match(source, /target\/module-sources\/tinyconnectors/);
  assert.match(source, /stage_release_module tinyconnectors/);
  assert.doesNotMatch(source, /cargo build/, "no hand-rolled module build");
  assert.doesNotMatch(source, /--target-dir|native-test-modules/);
  assert.doesNotMatch(
    source,
    /vendor\/tiny(?:juice|search|connectors)\/target/,
  );
});

test("local MCP registry E2E tests load the pinned TinyMCP module", () => {
  const source = readFileSync(localRustRunner, "utf8");
  assert.match(source, /stage_release_module tinymcp/);
  assert.match(source, /OPENHUMAN_MODULE_PATH="\$tinymcp_dir/);
});

test("linked TinyDocs E2E enables its required local feature", () => {
  const localSource = readFileSync(localRustRunner, "utf8");
  const localBranch = localSource.indexOf(
    'if [ "$target" = "tinydocs_linked_e2e" ]',
  );
  assert.ok(
    localBranch >= 0,
    "the Rust gate gives linked TinyDocs an explicit branch",
  );
  const localEnd = localSource.indexOf("\n    elif ", localBranch);
  assert.ok(
    localEnd > localBranch,
    "the linked target branch has a bounded body",
  );
  assert.match(
    localSource.slice(localBranch, localEnd),
    /--features linked-modules/,
  );

  const e2eSource = readFileSync(rustE2eRunner, "utf8");
  const e2eBranch = e2eSource.indexOf(
    'if [ "$suite" = "tinydocs_linked_e2e" ]',
  );
  assert.ok(
    e2eBranch >= 0,
    "the Rust E2E runner gives linked TinyDocs an explicit branch",
  );
  const e2eEnd = e2eSource.indexOf("\n  fi\n", e2eBranch);
  assert.ok(
    e2eEnd > e2eBranch,
    "the E2E linked target branch has a bounded body",
  );
  assert.match(
    e2eSource.slice(e2eBranch, e2eEnd),
    /--features "\$PRODUCT_FEATURES,linked-modules"/,
    "the E2E target runs with linked-modules in addition to product features",
  );
});

test("CI exports the staged TinyMCP library for the multi-server suite", () => {
  const source = readFileSync(installer, "utf8");
  assert.match(
    source,
    /echo "OPENHUMAN_TINYMCP_TEST_MODULE=\$\{tinymcp_artifact\}"/,
    "mcp_registry_multi_server loads TinyMCP by file, so the CI env file names it",
  );
});

test("MCP registry Rust integration tests run serially", () => {
  const source = readFileSync(localRustRunner, "utf8");
  assert.match(
    source,
    /mcp_registry_e2e[\s\S]*?--test-threads=1/,
    "the TinyMCP process state is shared across test cases",
  );
});

test("config/auth in-process integration tests run serially in both full runners", () => {
  for (const [runner, path] of [
    ["test-rust-with-mock.sh", localRustRunner],
    ["test-rust-e2e.sh", rustE2eRunner],
  ]) {
    const source = readFileSync(path, "utf8");
    const start = source.indexOf("run_in_process_modules() {");
    const end = source.indexOf("\n}", start);
    assert.ok(
      start >= 0 && end > start,
      `${runner} has an in-process module runner`,
    );
    assert.match(
      source.slice(start, end),
      /config_auth_app_state_connectivity_e2e[\s\S]*?--test-threads=1/,
      `${runner} serializes auth RPC fixtures that share process-wide listeners`,
    );
  }
});

test("multi-server MCP tests run serially through the module-backed host", () => {
  const source = readFileSync(localRustRunner, "utf8");
  const fullSuiteStart = source.indexOf(
    'elif [ "$target" = "mcp_registry_multi_server" ]',
  );
  assert.ok(
    fullSuiteStart >= 0,
    "full Rust gate gives the suite an isolated process",
  );
  const fullSuiteEnd = source.indexOf("\n    else\n", fullSuiteStart);
  assert.ok(
    fullSuiteEnd > fullSuiteStart,
    "full-suite branch has a bounded body",
  );
  assert.match(
    source.slice(fullSuiteStart, fullSuiteEnd),
    /--test-threads=1/,
    "parallel tests race through TinyMCP's process-wide module client",
  );
  assert.match(
    source.slice(fullSuiteStart, fullSuiteEnd),
    /OPENHUMAN_TINYMCP_TEST_MODULE="\$tinymcp_artifact"/,
    "the tests receive the staged TinyMCP library path",
  );

  const directStart = source.indexOf(
    'elif [ "$#" -ge 2 ] && [ "$1" = "--test" ] && [ "$2" = "mcp_registry_multi_server" ]',
  );
  assert.ok(
    directStart >= 0,
    "the focused runner gives the suite the same isolation",
  );
  const directEnd = source.indexOf("\nelse\n", directStart);
  assert.ok(directEnd > directStart, "focused branch has a bounded body");
  assert.match(
    source.slice(directStart, directEnd),
    /--test-threads=1/,
    "focused and full-suite runs must use the same serialization",
  );
  assert.match(
    source.slice(directStart, directEnd),
    /OPENHUMAN_TINYMCP_TEST_MODULE="\$tinymcp_artifact"/,
    "focused runs use the same staged TinyMCP artifact",
  );
});

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

for (const id of [
  "tinybox",
  "tinywallet",
  "tinychannels",
  "tinymcp",
  "tinyhosts",
]) {
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

for (const [runner, archiveVariable, directoryVariable] of [
  ["modules/stage-local-modules.sh", "archive", "dir"],
  ["test-rust-with-mock.sh", "tinybox_archive", "tinybox_dir"],
]) {
  test(`${runner} extracts ${archiveVariable} with the current user's ownership`, () => {
    const source = readFileSync(
      fileURLToPath(new URL(`../${runner}`, import.meta.url)),
      "utf8",
    );
    const extraction = source
      .split("\n")
      .find(
        (line) =>
          line.trimStart().startsWith("tar ") &&
          line.includes(`"$${archiveVariable}"`),
      );
    assert.ok(extraction, "runner must extract the pinned archive");
    const root = mkdtempSync(join(tmpdir(), "module-ownership-"));
    try {
      const payload = join(root, "payload");
      const destination = join(root, "extracted");
      mkdirSync(payload);
      mkdirSync(destination);
      writeFileSync(join(payload, "module.so"), "native module fixture");
      const archive = join(root, "module.tar.gz");
      const packed = spawnSync(
        "python3",
        [
          "-c",
          [
            "import sys, tarfile",
            "with tarfile.open(sys.argv[1], 'w:gz') as archive:",
            "    info = archive.gettarinfo(sys.argv[2], arcname='module.so')",
            "    info.uid = info.gid = 12345",
            "    info.uname = info.gname = ''",
            "    with open(sys.argv[2], 'rb') as payload:",
            "        archive.addfile(info, payload)",
          ].join("\n"),
          archive,
          join(payload, "module.so"),
        ],
        { encoding: "utf8" },
      );
      assert.equal(packed.status, 0, packed.stderr);
      // Force the superuser default even on unprivileged developer machines.
      // The runner's --no-same-owner must override it on the real tar command.
      const result = spawnSync(
        "bash",
        [
          "-c",
          ['tar() { command tar --same-owner "$@"; }', extraction].join("\n"),
        ],
        {
          encoding: "utf8",
          env: {
            ...process.env,
            [archiveVariable]: archive,
            [directoryVariable]: destination,
          },
        },
      );
      assert.equal(result.status, 0, result.stdout + result.stderr);
      assert.equal(
        statSync(join(destination, "module.so")).uid,
        statSync(destination).uid,
      );
      assert.equal(
        statSync(join(destination, "module.so")).gid,
        statSync(destination).gid,
      );
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });
}

for (const runner of ["test-rust-with-mock.sh", "test-rust-e2e.sh"]) {
  test(`${runner} supplies the TinyMCP artifact and executes ignored registry cases`, () => {
    const source = readFileSync(
      new URL(`../${runner}`, import.meta.url),
      "utf8",
    );
    const setupStart = source.indexOf(
      runner === "test-rust-with-mock.sh"
        ? "# The MCP registry E2E targets"
        : "e2e_source_ids=(",
    );
    assert.ok(setupStart >= 0, "runner prepares the pinned TinyMCP fixture");
    const setupEnd = source.indexOf("\nfi\n", setupStart) + 4;
    const setup = source.slice(setupStart, setupEnd);
    const stageLib = fileURLToPath(
      new URL("../modules/stage-local-modules.sh", import.meta.url),
    );
    const root = mkdtempSync(join(tmpdir(), "mcp-runner-"));
    try {
      const moduleDir = join(root, "target/test-modules/tinymcp/test-digest");
      mkdirSync(moduleDir, { recursive: true });
      writeFileSync(join(moduleDir, "libtinymcp_module.so"), "fixture");
      const boxDir = join(root, "target/test-modules/tinybox/test-digest");
      mkdirSync(boxDir, { recursive: true });
      writeFileSync(join(boxDir, "libtinybox.so"), "fixture");
      // Evaluate the real runner branches with only external commands stubbed.
      // A cached fixture avoids downloads while exercising the path export.
      const cargoStub = `cargo_test() {
        if [[ " $* " == *" --test mcp_registry_e2e "* ]]; then
          printf 'ARTIFACT=%s\\n' "\${OPENHUMAN_MODULE_PATH:-missing}"
          printf 'ARG=%s\\n' "$@"
        fi
      }`;
      let invocation;
      if (runner === "test-rust-with-mock.sh") {
        const start = source.indexOf("run_full_suite() {");
        const end = source.indexOf("\n}\n", start);
        const directStart = source.indexOf(
          'elif [ "$#" -ge 2 ] && [ "$1" = "--test" ] && [ "$2" = "mcp_registry_e2e" ]',
        );
        const directEnd = source.indexOf("\nelse\n", directStart);
        invocation = [
          "integration_test_targets() { echo mcp_registry_e2e; }",
          "run_build_only_reaper_test() { :; }",
          "run_tinychannels_linked_fixture_tests() { :; }",
          source.slice(start, end + 3),
          "run_full_suite",
          "set -- --test mcp_registry_e2e -- --nocapture",
          "if false; then :",
          source.slice(directStart, directEnd),
          "fi",
        ].join("\n");
      } else {
        const start = source.indexOf('  if [ "$suite" = "mcp_registry_e2e" ]');
        const end = source.indexOf('  if [ "$HAS_EXTRA_ARGS"', start);
        invocation = [
          "suite=mcp_registry_e2e",
          "CARGO_BIN=cargo_test",
          "EXTRA_ARGS=(--nocapture)",
          'bash() { shift; "$@"; }',
          "for once in 1; do",
          source.slice(start, end),
          "done",
        ].join("\n");
      }
      const result = spawnSync(
        "bash",
        [
          "-c",
          [
            "set -euo pipefail",
            'REPO_ROOT="$TEST_ROOT"',
            'SCRIPT_DIR="$TEST_ROOT"',
            "wallet_platform=ubuntu-22.04-x86_64",
            "module_platform=ubuntu-22.04-x86_64",
            "module_ext=so",
            "PRODUCT_FEATURES=mcp",
            `source ${JSON.stringify(stageLib)}`,
            "node() { printf '%s\\tunused\\tmodule.tar.gz\\ttest-digest\\n' \"${@: -1}\"; }",
            "unset OPENHUMAN_MODULE_PATH",
            setup,
            'connectors_module_path="/connectors:${OPENHUMAN_MODULE_PATH:-}"',
            "hosts_dir=/hosts",
            cargoStub,
            invocation,
          ].join("\n"),
        ],
        {
          encoding: "utf8",
          env: {
            ...process.env,
            TEST_ROOT: root,
            OPENHUMAN_MODULE_SOURCE: "",
          },
        },
      );
      assert.equal(result.status, 0, result.stdout + result.stderr);
      const runs = result.stdout.split("ARTIFACT=").slice(1);
      assert.equal(runs.length, runner === "test-rust-with-mock.sh" ? 2 : 1);
      for (const run of runs) {
        assert.ok(
          run.split("\n")[0].split(":").includes(moduleDir),
          "the pinned TinyMCP release directory is on the module search path",
        );
        assert.match(
          run,
          /^ARG=--ignored$/m,
          "ignored cases must actually execute",
        );
        assert.match(run, /^ARG=--test-threads=1$/m);
      }
      assert.match(
        runs.at(-1),
        /^ARG=--nocapture$/m,
        "caller arguments survive",
      );
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });
}

test("the Rust E2E runner reuses container-staged module paths", () => {
  const installerSource = readFileSync(installer, "utf8");
  const runnerSource = readFileSync(rustE2eRunner, "utf8");
  assert.match(installerSource, /OPENHUMAN_TEST_MODULES_STAGED=1/);
  assert.match(runnerSource, /OPENHUMAN_TEST_MODULES_STAGED/);
  assert.match(runnerSource, /pre-staged native modules/);
});

test("the CI installer builds nothing: every module, tinyconnectors included, is a registry release", () => {
  const source = readFileSync(installer, "utf8");
  assert.doesNotMatch(source, /cargo build|cargo-cancel|--target-dir/);
  assert.doesNotMatch(source, /stage-connector-test-module|modules\.toml/);
  const assets = resolveTestModuleAssets(
    readRegistrySource(),
    "ubuntu-22.04-x86_64",
  );
  const connectors = assets.find((asset) => asset.id === "tinyconnectors");
  assert.ok(
    connectors,
    "the installer resolves tinyconnectors from the registry",
  );
  assert.match(connectors.sha256, /^[a-f0-9]{64}$/);
  assert.match(connectors.url, /\/releases\/download\/v\d+\.\d+\.\d+\//);
});
