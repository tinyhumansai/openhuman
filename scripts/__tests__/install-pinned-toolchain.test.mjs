import assert from "node:assert/strict";
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const script = join(repoRoot, "scripts/ci/install-pinned-toolchain.sh");

/**
 * Run the script with fake rustup/cargo/rustc. The toolchain starts broken
 * (cargo fails) when `installsFailing > 0`, and the first `installsFailing`
 * installs leave it broken. `startBroken` overrides the initial state.
 */
function run(installsFailing, { startBroken = installsFailing > 0 } = {}) {
  const dir = mkdtempSync(join(tmpdir(), "pinned-toolchain-"));
  const bin = join(dir, "bin");
  mkdirSync(bin);
  const state = join(dir, "installs");
  writeFileSync(state, "0");
  if (startBroken) writeFileSync(join(dir, "broken"), "broken");
  writeFileSync(
    join(bin, "rustup"),
    `#!/bin/sh\nlog="${dir}/log"\necho "rustup $*" >> "$log"\n` +
      `if [ "$1 $2" = "component list" ]; then printf 'rustfmt-x\\nclippy-x\\nllvm-tools-x\\n'; fi\n` +
      `if [ "$1 $2" = "toolchain install" ]; then n=$(( $(cat "${state}") + 1 )); echo $n > "${state}"; ` +
      `[ $n -le ${installsFailing} ] && echo broken > "${dir}/broken" || rm -f "${dir}/broken"; fi\nexit 0\n`,
  );
  writeFileSync(join(bin, "cargo"), `#!/bin/sh\n[ -e "${dir}/broken" ] && exit 1\necho cargo 9.9.9\n`);
  writeFileSync(join(bin, "rustc"), `#!/bin/sh\necho rustc 9.9.9\n`);
  writeFileSync(join(bin, "sleep"), "#!/bin/sh\nexit 0\n");
  for (const f of ["rustup", "cargo", "rustc", "sleep"]) chmodSync(join(bin, f), 0o755);
  const r = spawnSync("bash", [script], {
    env: { ...process.env, PATH: `${bin}:${process.env.PATH}` },
    encoding: "utf8",
  });
  return { r, log: readFileSync(join(dir, "log"), "utf8") };
}

test("a working toolchain the image already ships is reused without reinstalling or uninstalling", () => {
  const { r, log } = run(0, { startBroken: false });
  assert.equal(r.status, 0, r.stderr);
  assert.doesNotMatch(log, /toolchain install /);
  assert.doesNotMatch(log, /toolchain uninstall /);
});

test("a toolchain install that leaves cargo broken is uninstalled and retried", () => {
  const { r, log } = run(1);
  assert.equal(r.status, 0, r.stderr);
  assert.match(log, /toolchain uninstall /);
  assert.equal(log.match(/toolchain install /g).length, 2);
});

test("the pinned toolchain install includes the llvm-tools component the coverage lanes need", () => {
  const { log } = run(0, { startBroken: true });
  assert.match(log, /toolchain install .*-c llvm-tools-preview/);
});

test("a toolchain that never installs cleanly fails the step", () => {
  const { r } = run(99);
  assert.notEqual(r.status, 0);
});

test("container lane jobs install the pinned toolchain before any lane runs", () => {
  const wf = readFileSync(join(repoRoot, ".github/workflows/ci-lanes.yml"), "utf8");
  const install = wf.indexOf("scripts/ci/install-pinned-toolchain.sh");
  const lanes = wf.indexOf("- name: Start lanes");
  assert.ok(lanes > 0, "ci-lanes.yml must still have a Start lanes step");
  assert.ok(install > 0 && install < lanes, "ci-lanes.yml must run install-pinned-toolchain.sh before Start lanes");
});
