#!/usr/bin/env node
// Drift guard: the toolchain installed by the openhuman_ci image, the version-pinned
// `dtolnay/rust-toolchain@<version>` steps used by the workflows that run on
// bare runners instead of the container, and the toolchain version installed
// by .github/Dockerfile must all match rust-toolchain.toml's pinned channel.
//
// Nothing enforces that agreement automatically — bumping rust-toolchain.toml
// without rebuilding/retagging the CI image (or vice versa) leaves jobs
// silently running a different compiler than local `cargo` invocations use,
// which is exactly the kind of drift that is easy to introduce in one PR and
// hard to notice until a toolchain-specific feature fails only in CI (or only
// locally). This script fails loudly instead.
//
// Usage: node scripts/ci/check-toolchain-image.mjs

import { readdirSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(__dirname, "..", "..");

function fail(message) {
  console.error(`[check-toolchain-image] ${message}`);
  process.exitCode = 1;
}

function readToolchainChannel() {
  const tomlPath = path.join(repoRoot, "rust-toolchain.toml");
  const text = readFileSync(tomlPath, "utf8");
  const match = text.match(/^channel\s*=\s*"([^"]+)"/m);
  if (!match) {
    fail(
      `could not find a "channel" line in ${path.relative(repoRoot, tomlPath)}`,
    );
    return null;
  }
  return match[1];
}

// Only version-pinned uses drift. `@stable` / `@master` track upstream on
// purpose and are none of this guard's business.
const ACTION_PIN_RE = /dtolnay\/rust-toolchain@([0-9][0-9A-Za-z.\-]*)/g;
// The other spelling: `uses: dtolnay/rust-toolchain@stable` with an explicit
// `toolchain: "1.2.3"` input (the mobile workflows use this form).
const TOOLCHAIN_INPUT_RE =
  /^\s*toolchain:\s*"?([0-9]+\.[0-9]+(?:\.[0-9]+)?)"?\s*$/gm;

// Scan the whole workflows directory rather than a hand-maintained file list:
// a new workflow that pins the image or the action is exactly the case this
// guard exists to catch, and it would slip through a fixed list unnoticed.
function workflowFiles() {
  const dir = path.join(repoRoot, ".github", "workflows");
  return readdirSync(dir)
    .filter((name) => name.endsWith(".yml") || name.endsWith(".yaml"))
    .sort()
    .map((name) => path.join(dir, name));
}

function findPinnedVersions(regex, kind) {
  const found = [];
  for (const filePath of workflowFiles()) {
    const text = readFileSync(filePath, "utf8");
    let match;
    regex.lastIndex = 0;
    while ((match = regex.exec(text)) !== null) {
      found.push({
        kind,
        file: path.relative(repoRoot, filePath),
        version: match[1],
        line: text.slice(0, match.index).split("\n").length,
      });
    }
  }
  return found;
}

function readDockerfileToolchainVersion() {
  const dockerfilePath = path.join(repoRoot, ".github", "Dockerfile");
  const text = readFileSync(dockerfilePath, "utf8");
  const match = text.match(/--default-toolchain\s+([0-9][0-9A-Za-z.\-]*)/);
  if (!match) {
    fail(
      `could not find "--default-toolchain <version>" in ${path.relative(repoRoot, dockerfilePath)}`,
    );
    return null;
  }
  return match[1];
}

function main() {
  const channel = readToolchainChannel();
  if (!channel) return;

  const actionPins = [
    ...findPinnedVersions(ACTION_PIN_RE, "dtolnay/rust-toolchain pin"),
    ...findPinnedVersions(
      TOOLCHAIN_INPUT_RE,
      'version-pinned "toolchain:" input',
    ),
  ];

  const dockerfileVersion = readDockerfileToolchainVersion();

  const mismatches = actionPins.filter((pin) => pin.version !== channel);
  for (const pin of mismatches) {
    fail(
      `${pin.file}:${pin.line} pins ${pin.kind} "${pin.version}", but rust-toolchain.toml channel is "${channel}"`,
    );
  }

  if (dockerfileVersion && dockerfileVersion !== channel) {
    fail(
      `.github/Dockerfile installs toolchain "${dockerfileVersion}", but rust-toolchain.toml channel is "${channel}"`,
    );
  }

  if (process.exitCode) {
    fail(
      "fix by bumping the drifted reference(s) to match rust-toolchain.toml, or bump rust-toolchain.toml and rebuild/retag the CI image (see build-ci-image.yml).",
    );
    return;
  }

  console.log(
    `[check-toolchain-image] OK — ${actionPins.length} version-pinned dtolnay/rust-toolchain step(s) and the Dockerfile ` +
      `toolchain all match rust-toolchain.toml channel "${channel}"`,
  );
}

main();
