import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const repoRoot = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
  "..",
);

test("CLI tarballs contain the core and standalone TUI", () => {
  const work = fs.mkdtempSync(path.join(os.tmpdir(), "openhuman-cli-package-"));
  const core = path.join(work, "core-fixture");
  const tui = path.join(work, "tui-fixture");
  const fakeBin = path.join(work, "bin");
  const nodeStub = path.join(fakeBin, "node");
  fs.writeFileSync(core, "core");
  fs.writeFileSync(tui, "tui");
  fs.mkdirSync(fakeBin);
  fs.writeFileSync(
    nodeStub,
    `#!/bin/sh
set -eu
test "$1" = "${path.join(repoRoot, "scripts/release/stage-modules.mjs")}" || exit 91
shift
while [ "$#" -gt 0 ]; do
  if [ "$1" = "--output" ]; then
    output="$2"
    shift 2
  else
    shift
  fi
done
mkdir -p "$output"
: > "$output/.gitkeep"
`,
    { mode: 0o755 },
  );

  execFileSync(
    "bash",
    [
      path.join(repoRoot, "scripts/release/package-cli-tarball.sh"),
      core,
      tui,
      "0.0.0",
      "x86_64-unknown-linux-gnu",
    ],
    {
      cwd: work,
      env: {
        ...process.env,
        PATH: `${fakeBin}${path.delimiter}${process.env.PATH}`,
        GITHUB_TOKEN: "",
      },
    },
  );

  const entries = execFileSync(
    "tar",
    [
      "-tzf",
      path.join(work, "openhuman-core-0.0.0-x86_64-unknown-linux-gnu.tar.gz"),
    ],
    { encoding: "utf8" },
  )
    .trim()
    .split("\n")
    .sort();
  assert.deepEqual(entries, [
    "bundled-modules/",
    "bundled-modules/.gitkeep",
    "openhuman-core",
    "openhuman-tui",
  ]);
});

test("package-manager consumers install and expose the TUI", () => {
  const formula = fs.readFileSync(
    path.join(repoRoot, "packages/homebrew/openhuman.rb"),
    "utf8",
  );
  assert.match(formula, /bin\.install "openhuman-core", "openhuman-tui"/);
  assert.match(formula, /system "#\{bin\}\/openhuman-tui", "--help"/);

  const apt = fs.readFileSync(
    path.join(repoRoot, "scripts/release/build-apt-packages.sh"),
    "utf8",
  );
  assert.match(apt, /openhuman-tui-amd64/);
  assert.match(apt, /openhuman-tui-arm64/);

  const deb = fs.readFileSync(
    path.join(repoRoot, "packages/deb/build.sh"),
    "utf8",
  );
  assert.match(deb, /usr\/bin\/openhuman-tui/);

  const npmPackage = JSON.parse(
    fs.readFileSync(path.join(repoRoot, "packages/npm/package.json"), "utf8"),
  );
  assert.equal(npmPackage.bin["openhuman-tui"], "./bin/openhuman-tui.js");

  const installer = fs.readFileSync(
    path.join(repoRoot, "packages/npm/install.js"),
    "utf8",
  );
  assert.match(installer, /openhuman-tui-bin/);
  assert.match(installer, /openhuman-tui\.exe/);
  assert.ok(
    fs.existsSync(path.join(repoRoot, "packages/npm/bin/openhuman-tui.js")),
  );
});

// packages/deb/build.sh shells out to dpkg-deb, which Debian-family hosts
// and the Linux CI runners have and macOS/Windows developer machines do not.
// Without it the build exits 127 and the red reads as a packaging defect.
function dpkgDebAvailable() {
  try {
    execFileSync("dpkg-deb", ["--version"], { stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
}

test("Debian packages install both commands", (t) => {
  if (!dpkgDebAvailable()) {
    t.skip("dpkg-deb not on PATH");
    return;
  }
  const work = fs.mkdtempSync(path.join(os.tmpdir(), "openhuman-deb-package-"));
  const core = path.join(work, "core-fixture");
  const tui = path.join(work, "tui-fixture");
  const fakeBin = path.join(work, "bin");
  const nodeStub = path.join(fakeBin, "node");
  fs.writeFileSync(core, "core");
  fs.writeFileSync(tui, "tui");
  fs.mkdirSync(fakeBin);
  fs.writeFileSync(
    nodeStub,
    `#!/bin/sh
set -eu
test "$1" = "${path.join(repoRoot, "scripts/release/stage-modules.mjs")}" || exit 91
shift
while [ "$#" -gt 0 ]; do
  if [ "$1" = "--output" ]; then
    output="$2"
    shift 2
  else
    shift
  fi
done
mkdir -p "$output"
: > "$output/.gitkeep"
`,
    { mode: 0o755 },
  );

  execFileSync(
    "bash",
    [
      "-c",
      'umask 077; exec "$@"',
      "bash",
      path.join(repoRoot, "packages/deb/build.sh"),
      core,
      tui,
      "0.0.0",
      "amd64",
    ],
    { cwd: work },
  );
  const contents = execFileSync(
    "dpkg-deb",
    ["--contents", path.join(work, "openhuman_0.0.0_amd64.deb")],
    { encoding: "utf8" },
  );
  assert.match(contents, /\.\/usr\/bin\/openhuman\n/);
  assert.match(contents, /\.\/usr\/bin\/openhuman-tui\n/);
});
