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
const MARKER = "openhuman-e2e-bundle.marker";
const BUNDLE_INPUTS = ["src/App.tsx", "public/favicon.ico", "index.html", "vite.config.ts"];

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
  const mockStarted = path.join(root, "mock-started");
  fs.mkdirSync(path.join(root, "app", "scripts"), { recursive: true });
  // Both scripts source the shared port helper (#5918).
  for (const file of [script, "e2e-ports.sh"]) {
    fs.copyFileSync(
      path.join(repoRoot, "app", "scripts", file),
      path.join(root, "app", "scripts", file),
    );
  }
  // e2e-web-session.sh checks these paths before starting the mock or core.
  // The fixture needs the same bundle inputs as the port-guard tests so this
  // suite reaches the guard under test instead of failing on its test tree.
  for (const rel of BUNDLE_INPUTS) {
    const file = path.join(root, "app", rel);
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, "");
  }
  fs.mkdirSync(path.join(root, "scripts"), { recursive: true });
  fs.copyFileSync(
    path.join(repoRoot, "scripts", "load-dotenv.sh"),
    path.join(root, "scripts", "load-dotenv.sh"),
  );

  // Every stub records that it ran, so a test can prove what was reached.
  const record = (name) => `echo "${name} $*" >> "${log}"`;

  // The mock backend "starts" and exits at once. Its launch is a background
  // job, so the health probe waits for the stub to record the launch before
  // reporting ready; otherwise a fast runner can race the child process.
  writeExecutable(
    path.join(bin, "node"),
    `#!/usr/bin/env bash
${record("node")}
if [[ "$*" == *"randomBytes"* ]]; then
  printf '%064d' 0
else
  touch "${mockStarted}"
fi
`,
  );
  writeExecutable(
    path.join(bin, "curl"),
    `#!/usr/bin/env bash
${record("curl")}
if [[ "$*" == *"/__admin/health"* ]]; then
  for _ in {1..500}; do
    [ -f "${mockStarted}" ] && exit 0
    sleep 0.01
  done
  exit 1
fi
`,
  );

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
  // Port availability belongs to e2e-web-port-guard.test.mjs. This fixture
  // starts no real listeners, so its probes must not inspect other sessions.
  writeExecutable(path.join(bin, "python3"), `#!/usr/bin/env bash\n${record("python3")}\nexit 0\n`);

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

function run(tree, script, extraEnv = {}) {
  const env = {
    ...process.env,
    PATH: `${tree.bin}:${process.env.PATH}`,
    // Do not let a developer's shell select real build tools or different
    // harness ports; the assertions exercise the fixed contract below.
    CARGO_BIN: path.join(tree.bin, "cargo"),
    E2E_MOCK_PORT: "18473",
    OPENHUMAN_CORE_PORT: "17788",
    RUST_HOST_TRIPLE: "test-triple",
    OPENHUMAN_WORKSPACE: path.join(tree.root, "workspace"),
    OPENHUMAN_KEYRING_MASTER_KEY: "11".repeat(32),
    E2E_WEB_CORE_TARGET_DIR: path.join(tree.root, "target"),
    ...extraEnv,
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
    assert.match(res.output, /pnpm --filter openhuman-app test:e2e:web:build/);
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

test("an explicit workspace requires its own keyring key", () => {
  const tree = makeTree("e2e-web-session.sh");
  try {
    const workspace = path.join(tree.root, "persistent-workspace");
    const res = run(tree, "e2e-web-session.sh", {
      OPENHUMAN_WORKSPACE: workspace,
      OPENHUMAN_E2E_ALLOW_EPHEMERAL_KEY: "",
      OPENHUMAN_KEYRING_MASTER_KEY: "",
      OPENHUMAN_KEYRING_MASTER_KEY_FILE: "",
    });

    assert.equal(res.status, 2, res.output);
    assert.match(res.output, /explicit OPENHUMAN_WORKSPACE requires/);
    assert.equal(fs.existsSync(workspace), false, "refuse before creating the supplied workspace");
  } finally {
    tree.cleanup();
  }
});

test("an ephemeral explicit workspace can generate a private key", () => {
  const tree = makeTree("e2e-web-session.sh");
  try {
    const distWeb = path.join(tree.root, "app", "dist-web");
    fs.mkdirSync(distWeb, { recursive: true });
    fs.writeFileSync(path.join(distWeb, MARKER), "VITE_OPENHUMAN_TARGET=web\n");
    fs.writeFileSync(
      path.join(distWeb, ".e2e-build-ports.json"),
      '{"e2e_mock_port":"18473","openhuman_core_port":"17788"}\n',
    );

    const res = run(tree, "e2e-web-session.sh", {
      OPENHUMAN_WORKSPACE: path.join(tree.root, "ci-workspace"),
      OPENHUMAN_E2E_ALLOW_EPHEMERAL_KEY: "1",
      OPENHUMAN_KEYRING_MASTER_KEY: "",
      OPENHUMAN_KEYRING_MASTER_KEY_FILE: "/host/master.key",
    });

    assert.match(res.output, /standalone core binary is missing/);
    assert.ok(
      tree.calls().some((call) => call.includes("randomBytes")),
      "an explicitly marked ephemeral workspace gets a fresh key",
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
    fs.writeFileSync(
      path.join(distWeb, ".e2e-build-ports.json"),
      '{"e2e_mock_port":"18473","openhuman_core_port":"17788"}\n',
    );

    const res = run(tree, "e2e-web-session.sh", {
      OPENHUMAN_WORKSPACE: "",
      OPENHUMAN_KEYRING_MASTER_KEY: "",
      OPENHUMAN_KEYRING_MASTER_KEY_FILE: "/host/master.key",
    });

    assert.doesNotMatch(res.output, /was not built for E2E/);
    assert.match(res.output, /standalone core binary is missing/);
    assert.ok(
      tree.calls().some((call) => call.includes("randomBytes")),
      "a disposable workspace gets a fresh key",
    );
    assert.ok(
      tree.calls().some((c) => c.includes("mock-api-server.mjs")),
      `expected the mock backend to be started:\n${tree.calls().join("\n")}`,
    );
  } finally {
    tree.cleanup();
  }
});

test("CI Full refreshes the E2E marker after restoring its content-keyed artifact", () => {
  const workflow = fs.readFileSync(
    path.join(repoRoot, ".github/workflows/ci-full.yml"),
    "utf8",
  );
  const restoreStep = workflow.match(
    /- name: Restore Playwright E2E artifact([\s\S]*?)(?=\n      - name: |\n    [a-zA-Z_-]+:|$)/,
  )?.[1];

  assert.ok(restoreStep, "CI Full must keep a Playwright artifact restore step");
  assert.match(restoreStep, /cp -a repo\/app\/dist-web app\//);
  assert.match(restoreStep, /touch app\/dist-web\/openhuman-e2e-bundle\.marker/);

  const cacheStep = workflow.match(
    /- name: Restore cached Playwright E2E artifact([\s\S]*?)(?=\n      - name: |\n    [a-zA-Z_-]+:|$)/,
  )?.[1];
  assert.ok(cacheStep, "CI Full must keep a Playwright artifact cache step");
  assert.match(cacheStep, /\.github\/workflows\/ci-full\.yml/);
  assert.match(
    cacheStep,
    /e2e-playwright-linux-c3c20d625bcc9f75e50c3c2c2b75c88d3e60a3f4352b0f19f5327b490438722d-/,
  );
  const jobBlock = (id, nextId) => {
    const start = workflow.indexOf(`  ${id}:\n`);
    const end = workflow.indexOf(`\n  ${nextId}:\n`, start);
    return start < 0 || end < 0
      ? undefined
      : workflow.slice(start, end);
  };
  const pinnedImage =
    "image: ghcr.io/tinyhumansai/openhuman_ci:latest@sha256:c3c20d625bcc9f75e50c3c2c2b75c88d3e60a3f4352b0f19f5327b490438722d";
  const producerJob = jobBlock("build-playwright-e2e-artifact", "playwright-e2e");
  const consumerJob = jobBlock("playwright-e2e", "e2e-desktop");
  assert.ok(producerJob, "Playwright artifact producer job must exist");
  assert.ok(consumerJob, "Playwright artifact consumer job must exist");
  assert.ok(
    producerJob.includes(pinnedImage),
    "Playwright artifact producer must use the cache-key CI image",
  );
  assert.ok(
    consumerJob.includes(pinnedImage),
    "Playwright artifact consumer must use the cache-key CI image",
  );
  assert.equal(
    producerJob.match(/container:\n\s+image: .+/)?.[0],
    consumerJob.match(/container:\n\s+image: .+/)?.[0],
    "Playwright artifact producer and consumer must use the same pinned CI image",
  );
  assert.match(cacheStep, /crates\/\*\*/);
  assert.match(cacheStep, /vendor\/\*\*/);
  assert.match(cacheStep, /build\.rs/);
  assert.match(cacheStep, /\.cargo\/\*\*/);
  assert.match(cacheStep, /scripts\/ci\/product-features\.\*/);
  assert.match(cacheStep, /app\/tsconfig\*\.json/);
  assert.match(cacheStep, /packages\/\*\*/);
  assert.match(cacheStep, /pnpm-workspace\.yaml/);
  assert.match(cacheStep, /'package\.json'/);
  assert.match(cacheStep, /app\/scripts\/e2e-ports\.sh/);
});

test("e2e-web-build.sh marks the bundle it builds, recording the E2E settings", () => {
  const tree = makeTree("e2e-web-build.sh");
  try {
    fs.writeFileSync(
      path.join(tree.root, ".env"),
      "E2E_MOCK_PORT=28473\nOPENHUMAN_CORE_PORT=27788\n",
    );
    const res = run(tree, "e2e-web-build.sh");
    assert.equal(res.status, 0, res.output);

    const marker = path.join(tree.root, "app", "dist-web", MARKER);
    assert.ok(fs.existsSync(marker), `no ${MARKER} after the E2E build:\n${res.output}`);
    const recorded = fs.readFileSync(marker, "utf8");
    assert.match(recorded, /^VITE_OPENHUMAN_TARGET=web$/m);
    assert.match(recorded, /^VITE_OPENHUMAN_E2E_DEFAULT_CORE_MODE=cloud$/m);
    assert.match(recorded, /^VITE_BACKEND_URL=http:\/\/127\.0\.0\.1:18473$/m);
    assert.match(recorded, /^VITE_OPENHUMAN_CORE_RPC_URL=http:\/\/127\.0\.0\.1:17788\/rpc$/m);
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
