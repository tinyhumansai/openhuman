// The container lanes install jq, tomli and unzip from Ubuntu's archive. Pins
// that an unreachable archive falls back to another mirror, that a host which
// already carries the tools makes no apt call, and that the workflow uses the
// script instead of a bare `apt-get`.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const repoRoot = path.join(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const script = path.join(repoRoot, "scripts", "ci", "install-ci-packages.sh");

/** A fake host: stub commands on PATH, an apt sources file, a call log. */
function fakeHost({ present = [], reachable = [] } = {}) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "oh-ci-packages-"));
  const bin = path.join(dir, "bin");
  fs.mkdirSync(bin);
  const sources = path.join(dir, "sources.list");
  fs.writeFileSync(sources, "deb http://archive.ubuntu.com/ubuntu/ jammy main universe\n");
  const calls = path.join(dir, "calls");
  const stub = (name, body) => {
    fs.writeFileSync(path.join(bin, name), `#!/usr/bin/env bash\n${body}\n`, { mode: 0o755 });
  };
  for (const tool of present) stub(tool, "exit 0");
  if (!present.includes("python3")) stub("python3", "exit 1");
  // apt-get update always "succeeds", as the real one does when every index
  // fails to download; install only works once a reachable mirror is listed.
  stub(
    "apt-get",
    `echo "apt-get $*" >> ${JSON.stringify(calls)}
if [ "$1" = install ]; then
  for host in ${reachable.join(" ")}; do
    grep -q "$host" ${JSON.stringify(sources)} && exit 0
  done
  echo "E: Unable to locate package unzip" >&2
  exit 100
fi
exit 0`,
  );
  const run = () =>
    spawnSync("bash", [script], {
      encoding: "utf8",
      env: {
        PATH: `${bin}:/usr/bin:/bin`,
        CI_APT_SOURCES: sources,
        CI_APT_RETRY_DELAY: "0",
      },
    });
  const log = () => (fs.existsSync(calls) ? fs.readFileSync(calls, "utf8").trim().split("\n") : []);
  return { run, log, sources };
}

test("an unreachable Ubuntu archive falls back to another mirror", () => {
  const host = fakeHost({ reachable: ["azure.archive.ubuntu.com"] });
  const result = host.run();
  assert.equal(result.status, 0, result.stderr);
  assert.match(fs.readFileSync(host.sources, "utf8"), /azure\.archive\.ubuntu\.com/);
  assert.ok(host.log().filter((call) => call.startsWith("apt-get install")).length >= 2);
});

test("a host that already carries the tools makes no apt call", () => {
  const host = fakeHost({ present: ["jq", "unzip", "python3"] });
  const result = host.run();
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(host.log(), []);
});

test("no reachable mirror fails the step", () => {
  const host = fakeHost({ reachable: [] });
  const result = host.run();
  assert.notEqual(result.status, 0);
});

test("the container lanes install their tools through the script", () => {
  const workflow = fs.readFileSync(path.join(repoRoot, ".github", "workflows", "ci-lanes.yml"), "utf8");
  const step = workflow.match(
    /- name: Install jq and unzip for release-script tests\n(?:\s+#.*\n)*\s+if: .+\n\s+run: (.+)\n/,
  );
  assert.ok(step, "the install step exists");
  assert.equal(step[1].trim(), "bash scripts/ci/install-ci-packages.sh");
});
