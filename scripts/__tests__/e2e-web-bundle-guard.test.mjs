// Regression tests for the web E2E bundle guard (#5920).
//
// `pnpm build:web` exits 0 with a bundle that points at the wrong backend and
// has the E2E affordances compiled out. e2e-web-session.sh used to serve it
// anyway, and every Playwright spec then failed as though the product had
// regressed. e2e-web-build.sh now marks the bundles it builds, and the session
// refuses to start without that mark.
//
// These run the REAL scripts, copied into a temporary tree so that their
// `SCRIPT_DIR`/`APP_DIR`/`REPO_ROOT` resolve there, with every external command
// they reach (node, curl, pnpm, rustc, cargo) replaced by a stub on PATH. No
// Rust build, pnpm install or network is needed.

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
const MARKER = ".openhuman-e2e-bundle";

function writeExecutable(file, body) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, body, { mode: 0o755 });
}

/**
 * A temporary checkout holding a copy of one real app script, plus stubs.
 *
 * @param {string} script path of the script under app/scripts/
 */
function makeTree(script) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "e2e-web-bundle-guard-"));
  const bin = path.join(root, "bin");
  const log = path.join(root, "calls.log");
  fs.mkdirSync(path.join(root, "app", "scripts"), { recursive: true });
  fs.copyFileSync(
    path.join(repoRoot, "app", "scripts", script),
    path.join(root, "app", "scripts", script),
  );

  // Every stub records that it ran, so a test can prove what was reached.
  const record = (name) => `echo "${name} $*" >> "${log}"`;

  // The mock backend "starts" and exits at once; curl reports every health
  // probe as ready, so the session moves straight on to its next check.
  writeExecutable(path.join(bin, "node"), `#!/usr/bin/env bash\n${record("node")}\n`);
  writeExecutable(path.join(bin, "curl"), `#!/usr/bin/env bash\n${record("curl")}\n`);

  // `pnpm run build:web` behaves like Vite's `emptyOutDir: true`: it replaces
  // dist-web wholesale, taking any marker from a previous build with it.
  writeExecutable(
    path.join(bin, "pnpm"),
    `#!/usr/bin/env bash
${record("pnpm")}
if [ "$1 $2" = "run build:web" ]; then
  rm -rf dist-web && mkdir -p dist-web && echo '<!doctype html>' > dist-web/index.html
fi
`,
  );
  writeExecutable(path.join(bin, "rustc"), `#!/usr/bin/env bash\necho 'host: test-triple'\n`);
  writeExecutable(path.join(bin, "cargo"), `#!/usr/bin/env bash\n${record("cargo")}\n`);

  // The repo-level helpers e2e-web-build.sh shells out to.
  writeExecutable(
    path.join(root, "scripts", "ci", "product-features.sh"),
    "#!/usr/bin/env bash\necho voice\n",
  );
  writeExecutable(
    path.join(root, "scripts", "ci-cancel-aware.sh"),
    '#!/usr/bin/env bash\nexec "$@"\n',
  );

  const calls = () =>
    fs.existsSync(log)
      ? fs.readFileSync(log, "utf8").split("\n").filter(Boolean)
      : [];
  const cleanup = () => fs.rmSync(root, { recursive: true, force: true });
  return { root, bin, calls, cleanup };
}

function run(tree, script) {
  const env = {
    ...process.env,
    PATH: `${tree.bin}:${process.env.PATH}`,
    RUST_HOST_TRIPLE: "test-triple",
    OPENHUMAN_WORKSPACE: path.join(tree.root, "workspace"),
    E2E_WEB_CORE_TARGET_DIR: path.join(tree.root, "target"),
  };
  const file = path.join(tree.root, "app", "scripts", script);
  try {
    const output = execFileSync("bash", [file], {
      encoding: "utf8",
      env,
      stdio: ["ignore", "pipe", "pipe"],
    });
    return { status: 0, output };
  } catch (err) {
    return { status: err.status, output: `${err.stdout ?? ""}${err.stderr ?? ""}` };
  }
}

test("the session refuses a dist-web that e2e-web-build.sh did not produce", () => {
  // Exactly #5920: a bundle is present — as after `pnpm build:web` — but it
  // carries no E2E marker.
  const tree = makeTree("e2e-web-session.sh");
  try {
    fs.mkdirSync(path.join(tree.root, "app", "dist-web"), { recursive: true });
    fs.writeFileSync(path.join(tree.root, "app", "dist-web", "index.html"), "<!doctype html>");

    const res = run(tree, "e2e-web-session.sh");

    assert.equal(res.status, 1, res.output);
    assert.match(res.output, /was not built for E2E/);
    assert.match(res.output, /pnpm test:e2e:web:build/);
    // Refused before anything was started, not after.
    assert.deepEqual(
      tree.calls().filter((c) => c.startsWith("node ")),
      [],
      "the mock backend must not be started for a bundle that will be refused",
    );
  } finally {
    tree.cleanup();
  }
});

test("the session accepts a marked bundle and proceeds past the guard", () => {
  // Pins that the guard is not simply always-fail. With the marker present the
  // session starts the mock backend and reaches its next precondition — the
  // standalone core binary, deliberately absent here.
  const tree = makeTree("e2e-web-session.sh");
  try {
    const distWeb = path.join(tree.root, "app", "dist-web");
    fs.mkdirSync(distWeb, { recursive: true });
    fs.writeFileSync(path.join(distWeb, MARKER), "VITE_OPENHUMAN_TARGET=web\n");

    const res = run(tree, "e2e-web-session.sh");

    assert.doesNotMatch(res.output, /was not built for E2E/);
    assert.match(res.output, /standalone core binary is missing/);
    assert.ok(
      tree.calls().some((c) => c.includes("mock-api-server.mjs")),
      `expected the mock backend to be started:\n${tree.calls().join("\n")}`,
    );
  } finally {
    tree.cleanup();
  }
});

test("e2e-web-build.sh marks the bundle it builds, recording the E2E settings", () => {
  const tree = makeTree("e2e-web-build.sh");
  try {
    const res = run(tree, "e2e-web-build.sh");
    assert.equal(res.status, 0, res.output);

    const marker = path.join(tree.root, "app", "dist-web", MARKER);
    assert.ok(fs.existsSync(marker), `no ${MARKER} after the E2E build:\n${res.output}`);
    const recorded = fs.readFileSync(marker, "utf8");
    assert.match(recorded, /^VITE_OPENHUMAN_TARGET=web$/m);
    assert.match(recorded, /^VITE_OPENHUMAN_E2E_DEFAULT_CORE_MODE=cloud$/m);
    assert.match(recorded, /^VITE_BACKEND_URL=http:\/\/127\.0\.0\.1:18473$/m);
  } finally {
    tree.cleanup();
  }
});

test("a later plain build:web leaves no marker, so the session refuses it", () => {
  // The whole mechanism rests on this: an E2E build followed by an ordinary
  // `pnpm build:web` must not keep the old marker next to the new bundle.
  const tree = makeTree("e2e-web-build.sh");
  try {
    assert.equal(run(tree, "e2e-web-build.sh").status, 0);
    const appDir = path.join(tree.root, "app");
    const marker = path.join(appDir, "dist-web", MARKER);
    // Precondition, so this cannot pass against a build that never marked anything.
    assert.ok(fs.existsSync(marker), "the E2E build should have left a marker");

    execFileSync(path.join(tree.bin, "pnpm"), ["run", "build:web"], { cwd: appDir });

    assert.equal(fs.existsSync(marker), false, "a plain build:web must not keep the E2E marker");
  } finally {
    tree.cleanup();
  }
});

test("the web build empties its output directory, which the marker relies on", () => {
  // The stub above models `emptyOutDir: true`. If the real config stopped
  // emptying dist-web, a stale marker would survive a plain build and the guard
  // would wave through exactly the bundle it exists to refuse.
  const configs = fs
    .readdirSync(path.join(repoRoot, "app"))
    .filter((f) => /^vite\.config\.[cm]?[jt]s$/.test(f));
  assert.equal(configs.length, 1, `expected one app/vite.config.*, found: ${configs}`);
  const source = fs.readFileSync(path.join(repoRoot, "app", configs[0]), "utf8");
  assert.match(source, /outDir:\s*isWebTarget\s*\?\s*["']\.\.\/dist-web["']/);
  assert.match(source, /emptyOutDir:\s*true/);
});
