// Regression tests for the web E2E lane's port handling (#5918).
//
// The three harness ports were fixed constants and nothing checked whether they
// were free. A second session on the same machine bound none of them, and every
// readiness probe — plain HTTP GETs — was answered by the first session's mock,
// core and served bundle. The specs then ran against another build's backend
// state and failed as though the product had regressed.
//
// These run the REAL scripts, copied into a temporary tree so their
// `SCRIPT_DIR`/`APP_DIR`/`REPO_ROOT` resolve there, with the external commands
// they reach replaced by stubs on PATH. The ports, though, are real: each test
// that needs a taken port opens an actual listener on it.

import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const repoRoot = path.join(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const MARKER = "openhuman-e2e-bundle.marker";
const APP_SCRIPTS = ["e2e-ports.sh", "e2e-web-session.sh", "e2e-web-build.sh"];
// What the session stats to decide the bundle is not older than its sources
// (#5919). The fake checkout has to carry them or the gate refuses every tree
// here for a missing input instead of exercising what the test is about.
const BUNDLE_INPUTS = ["src/App.tsx", "public/favicon.ico", "index.html", "vite.config.ts"];

function writeExecutable(file, body) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, body, { mode: 0o755 });
}

/** A temporary checkout holding the real web E2E scripts, plus stubs. */
function makeTree() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "e2e-web-ports-"));
  const bin = path.join(root, "bin");
  const log = path.join(root, "calls.log");
  fs.mkdirSync(path.join(root, "app", "scripts"), { recursive: true });
  for (const script of APP_SCRIPTS) {
    fs.copyFileSync(
      path.join(repoRoot, "app", "scripts", script),
      path.join(root, "app", "scripts", script),
    );
  }
  fs.mkdirSync(path.join(root, "scripts"), { recursive: true });
  fs.copyFileSync(
    path.join(repoRoot, "scripts", "load-dotenv.sh"),
    path.join(root, "scripts", "load-dotenv.sh"),
  );

  for (const rel of BUNDLE_INPUTS) {
    const file = path.join(root, "app", rel);
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, "");
  }

  const record = (name) => `echo "${name} $*" >> "${log}"`;

  // The mock backend "starts" and exits at once, so the health probe waits for
  // the stub to record the launch rather than racing the child process.
  writeExecutable(
    path.join(bin, "node"),
    `#!/usr/bin/env bash
${record("node")}
if [[ "$*" == *"randomBytes"* ]]; then
  printf '%064d' 0
fi
`,
  );
  writeExecutable(
    path.join(bin, "curl"),
    `#!/usr/bin/env bash
${record("curl")}
url="$2"
if [[ "$url" == *"/__admin/health" ]]; then
  for _ in {1..100}; do
    grep -q 'mock-api-server.mjs' "${log}" && exit 0
    sleep 0.01
  done
  exit 1
fi
if [[ "$url" == *"/health" ]]; then
  for _ in {1..100}; do
    [[ -f "$OPENHUMAN_WORKSPACE/core.started" ]] && exit 0
    sleep 0.01
  done
  exit 1
fi
`,
  );
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
  writeExecutable(
    path.join(root, "scripts", "ci", "product-features.sh"),
    "#!/usr/bin/env bash\necho voice\n",
  );
  writeExecutable(
    path.join(root, "scripts", "ci-cancel-aware.sh"),
    '#!/usr/bin/env bash\nexec "$@"\n',
  );

  const calls = () =>
    fs.existsSync(log) ? fs.readFileSync(log, "utf8").split("\n").filter(Boolean) : [];
  const cleanup = () => fs.rmSync(root, { recursive: true, force: true });
  return { root, bin, calls, cleanup };
}

/**
 * Install a stub `openhuman-core` that reports binding `boundPort`.
 *
 * The real core does not exit when its port is held: it falls back to a
 * neighbouring one and keeps running, so the session reads the address it
 * logs rather than trusting an HTTP probe.
 */
function stubCore(tree, boundPort, startupDelayMs = 0) {
  const bin = path.join(tree.root, "target", "debug", "openhuman-core");
  writeExecutable(
    bin,
    `#!/usr/bin/env bash
sleep ${startupDelayMs / 1000}
touch "$OPENHUMAN_WORKSPACE/core.started"
${boundPort == null ? "" : `echo "[core] OpenHuman core is ready \u2014 listening on http://127.0.0.1:${boundPort} (version test)"`}
sleep 30
`,
  );
  return bin;
}

/** Mark dist-web as an E2E bundle built for these ports. */
function markBundle(tree, { mockPort, corePort }) {
  const distWeb = path.join(tree.root, "app", "dist-web");
  fs.mkdirSync(distWeb, { recursive: true });
  fs.writeFileSync(path.join(distWeb, MARKER), "VITE_OPENHUMAN_TARGET=web\n");
  fs.writeFileSync(
    path.join(distWeb, ".e2e-build-ports.json"),
    `{"e2e_mock_port":"${mockPort}","openhuman_core_port":"${corePort}"}\n`,
  );
  backdateBundleInputs(tree);
}

function setMtimeAfter(file, reference) {
  // CI filesystems can have coarse timestamp granularity. Put the input two
  // seconds past the marker so find -newer exercises the stale-bundle path
  // deterministically even when both files were created in one test tick.
  const after = (fs.statSync(reference).mtimeMs + 2000) / 1000;
  fs.utimesSync(file, after, after);
}

/**
 * Age every bundle input so the marker is strictly newer, the way a real build
 * leaves them: `build:web` reads the sources and the marker is written after it
 * returns. Directories come last because writing a file inside one bumps its
 * mtime.
 */
function backdateBundleInputs(tree) {
  const marker = path.join(tree.root, "app", "dist-web", MARKER);
  const when = fs.statSync(marker).mtimeMs / 1000 - 10;
  for (const rel of [...BUNDLE_INPUTS, "src", "public"]) {
    fs.utimesSync(path.join(tree.root, "app", rel), when, when);
  }
}

function run(tree, script, env = {}) {
  const file = path.join(tree.root, "app", "scripts", script);
  try {
    const output = execFileSync("bash", [file], {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
      env: {
        ...process.env,
        PATH: `${tree.bin}:${process.env.PATH}`,
        CARGO_BIN: path.join(tree.bin, "cargo"),
        RUST_HOST_TRIPLE: "test-triple",
        OPENHUMAN_WORKSPACE: path.join(tree.root, "workspace"),
        OPENHUMAN_KEYRING_MASTER_KEY: "11".repeat(32),
        E2E_WEB_CORE_TARGET_DIR: path.join(tree.root, "target"),
        // Deliberately unset so each test states the ports it is exercising;
        // a developer's shell must not choose them.
        E2E_PORT_BASE: "",
        E2E_MOCK_PORT: "",
        OPENHUMAN_CORE_PORT: "",
        E2E_WEB_PORT: "",
        ...env,
      },
    });
    return { status: 0, output };
  } catch (err) {
    return { status: err.status, output: `${err.stdout ?? ""}${err.stderr ?? ""}` };
  }
}

/** Hold a real listener on `port` for the duration of `body`. */
async function whileListening(port, body, host = "127.0.0.1") {
  const server = net.createServer();
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(port, host, resolve);
  });
  try {
    return await body();
  } finally {
    await new Promise((resolve) => server.close(resolve));
  }
}

/** A port nothing is listening on right now. */
async function freePort() {
  const server = net.createServer();
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const { port } = server.address();
  await new Promise((resolve) => server.close(resolve));
  return port;
}

/**
 * A base whose whole block — base, base+1, base+2 — is free.
 *
 * An ephemeral port says nothing about its two neighbours, and this suite
 * asserts on which port a refusal names, so the block has to be clean.
 */
async function freeBase() {
  for (let attempt = 0; attempt < 50; attempt += 1) {
    const base = 20000 + Math.floor(Math.random() * 20000);
    const servers = [];
    try {
      for (const port of [base, base + 1, base + 2]) {
        const server = net.createServer();
        await new Promise((resolve, reject) => {
          server.once("error", reject);
          server.listen(port, "127.0.0.1", resolve);
        });
        servers.push(server);
      }
      return base;
    } catch {
      continue;
    } finally {
      for (const server of servers) await new Promise((resolve) => server.close(resolve));
    }
  }
  throw new Error("no free port block found");
}

/** Wait for a stubbed command to record `needle`, which it may do after exit. */
function waitForCall(tree, needle, timeoutMs = 5000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (tree.calls().some((c) => c.includes(needle))) return true;
    // The launch is a background job the session never waits on, so busy-wait
    // rather than assert on a process that may not be scheduled yet.
    execFileSync("sleep", ["0.05"]);
  }
  return false;
}

// ── the occupied-port guard ────────────────────────────────────────────────

for (const [name, offset] of [
  ["mock backend", 0],
  ["core", 1],
  ["web host", 2],
]) {
  test(`the session refuses to start when the ${name} port is already taken`, async () => {
    // The defect itself: the second session used to bind nothing here and run
    // its specs against the first session's services.
    const tree = makeTree();
    try {
      const base = await freeBase();
      markBundle(tree, { mockPort: base, corePort: base + 1 });

      const res = await whileListening(base + offset, () =>
        run(tree, "e2e-web-session.sh", { E2E_PORT_BASE: String(base) }),
      );

      assert.equal(res.status, 1, res.output);
      assert.match(res.output, new RegExp(`already in use on 127\\.0\\.0\\.1:.*${base + offset}`));
      assert.match(res.output, /E2E_PORT_BASE=<free base>/);
      assert.deepEqual(
        tree.calls().filter((c) => c.startsWith("node ")),
        [],
        "nothing may be started once a port is known to be taken",
      );
    } finally {
      tree.cleanup();
    }
  });
}

test("the session starts when all three ports are free", async () => {
  // Pins that the guard is not simply always-fail: with the block free the
  // session starts the mock and reaches its next precondition, the standalone
  // core binary, which is deliberately absent here.
  const tree = makeTree();
  try {
    const base = await freeBase();
    markBundle(tree, { mockPort: base, corePort: base + 1 });

    const res = run(tree, "e2e-web-session.sh", { E2E_PORT_BASE: String(base) });

    assert.doesNotMatch(res.output, /already in use/);
    assert.match(res.output, /standalone core binary is missing/);
    assert.ok(
      waitForCall(tree, "mock-api-server.mjs"),
      `expected the mock backend to be started:\n${tree.calls().join("\n")}`,
    );
  } finally {
    tree.cleanup();
  }
});

test("a taken port is reported even when another server answers its probe", async () => {
  // Why an HTTP probe cannot be the check: the occupant answers it. This is the
  // whole mechanism of the bug, so assert on it directly.
  const tree = makeTree();
  try {
    const base = await freeBase();
    markBundle(tree, { mockPort: base, corePort: base + 1 });

    const res = await whileListening(base, () => {
      // The stub curl reports the mock healthy as soon as a launch is recorded,
      // exactly as a real curl would be answered by the occupant.
      return run(tree, "e2e-web-session.sh", { E2E_PORT_BASE: String(base) });
    });

    assert.equal(res.status, 1, res.output);
    assert.match(res.output, /would run this session's specs against that one's backend/);
  } finally {
    tree.cleanup();
  }
});

// ── deriving a block from E2E_PORT_BASE ────────────────────────────────────

test("E2E_PORT_BASE gives the session base, base+1 and base+2", async () => {
  const tree = makeTree();
  try {
    const base = await freeBase();
    // Built for the derived mock/core ports, so the #6478 bundle-port guard
    // passes only if the session derived the same pair.
    markBundle(tree, { mockPort: base, corePort: base + 1 });

    const res = run(tree, "e2e-web-session.sh", { E2E_PORT_BASE: String(base) });

    assert.doesNotMatch(res.output, /dist-web was built for E2E_MOCK_PORT/);
    assert.match(res.output, /standalone core binary is missing/);
  } finally {
    tree.cleanup();
  }
});

test("an explicit E2E_MOCK_PORT still wins over the base", async () => {
  // CI lanes and developers set individual ports today; the base only supplies
  // defaults, so those callers keep working.
  const tree = makeTree();
  try {
    const base = await freeBase();
    const mockPort = await freePort();
    markBundle(tree, { mockPort, corePort: base + 1 });

    const res = run(tree, "e2e-web-session.sh", {
      E2E_PORT_BASE: String(base),
      E2E_MOCK_PORT: String(mockPort),
    });

    assert.doesNotMatch(res.output, /dist-web was built for E2E_MOCK_PORT/);
    assert.match(res.output, /standalone core binary is missing/);
  } finally {
    tree.cleanup();
  }
});

test("the default block is unchanged when no base is set", () => {
  // The recorded ports are the contract between the build and the session, so
  // read them from a real build rather than from the script's source.
  const tree = makeTree();
  try {
    assert.equal(run(tree, "e2e-web-build.sh").status, 0);
    const recorded = JSON.parse(
      fs.readFileSync(path.join(tree.root, "app", "dist-web", ".e2e-build-ports.json"), "utf8"),
    );
    assert.deepEqual(recorded.e2e_mock_port, "18473");
    assert.deepEqual(recorded.openhuman_core_port, "17788");
  } finally {
    tree.cleanup();
  }
});

test("an unusable E2E_PORT_BASE is rejected, not silently ignored", () => {
  const tree = makeTree();
  try {
    // `031000` would resolve base+1 as octal 12801 and `08000` would fail as an
    // invalid octal digit, both after passing a decimal range check.
    for (const base of ["abc", "80", "65534", "031000", "08000"]) {
      const res = run(tree, "e2e-web-session.sh", { E2E_PORT_BASE: base });
      assert.equal(res.status, 1, `E2E_PORT_BASE=${base} should be refused: ${res.output}`);
      assert.match(res.output, /E2E_PORT_BASE must be/);
    }
  } finally {
    tree.cleanup();
  }
});

// ── the build and the session must agree ───────────────────────────────────

test("the build bakes the derived ports, so the same base passes the session's check", async () => {
  const tree = makeTree();
  try {
    const base = await freeBase();
    assert.equal(run(tree, "e2e-web-build.sh", { E2E_PORT_BASE: String(base) }).status, 0);

    const recorded = JSON.parse(
      fs.readFileSync(path.join(tree.root, "app", "dist-web", ".e2e-build-ports.json"), "utf8"),
    );
    assert.equal(recorded.e2e_mock_port, String(base));
    assert.equal(recorded.openhuman_core_port, String(base + 1));
    assert.equal(recorded.vite_backend_url, `http://127.0.0.1:${base}`);

    const res = run(tree, "e2e-web-session.sh", { E2E_PORT_BASE: String(base) });
    assert.doesNotMatch(res.output, /dist-web was built for E2E_MOCK_PORT/);
    assert.match(res.output, /standalone core binary is missing/);
  } finally {
    tree.cleanup();
  }
});

test("a session on a different base than the build is refused", async () => {
  // The backend URL is compiled into the bundle, so a mismatch has to fail
  // loudly rather than serve a bundle that calls a mock nobody is running.
  const tree = makeTree();
  try {
    const built = await freeBase();
    const other = await freeBase();
    assert.equal(run(tree, "e2e-web-build.sh", { E2E_PORT_BASE: String(built) }).status, 0);

    const res = run(tree, "e2e-web-session.sh", { E2E_PORT_BASE: String(other) });

    assert.equal(res.status, 1, res.output);
    assert.match(res.output, /dist-web was built for E2E_MOCK_PORT/);
  } finally {
    tree.cleanup();
  }
});

test("a session with a different core port than the build is refused", async () => {
  const tree = makeTree();
  try {
    const base = await freeBase();
    markBundle(tree, { mockPort: base, corePort: base + 1 });

    const res = run(tree, "e2e-web-session.sh", {
      E2E_PORT_BASE: String(base),
      OPENHUMAN_CORE_PORT: String(base + 9),
    });

    assert.equal(res.status, 1, res.output);
    assert.match(res.output, /built for OPENHUMAN_CORE_PORT=/);
  } finally {
    tree.cleanup();
  }
});

test("a wildcard listener on one of the ports is still detected", async () => {
  // A bind probe would call this port free: with SO_REUSEADDR a 127.0.0.1 bind
  // coexists with an existing 0.0.0.0 listener on macOS and BSD, while that
  // listener keeps answering every HTTP probe the session makes.
  const tree = makeTree();
  try {
    const base = await freeBase();
    markBundle(tree, { mockPort: base, corePort: base + 1 });

    const res = await whileListening(
      base,
      () => run(tree, "e2e-web-session.sh", { E2E_PORT_BASE: String(base) }),
      "0.0.0.0",
    );

    assert.equal(res.status, 1, res.output);
    assert.match(res.output, new RegExp(`already in use on 127\\.0\\.0\\.1:.*${base}`));
  } finally {
    tree.cleanup();
  }
});

test("a second session cannot acquire the same port block", async () => {
  const tree = makeTree();
  try {
    const base = await freeBase();
    markBundle(tree, { mockPort: base, corePort: base + 1 });
    const lock = path.join(tree.root, `openhuman-e2e-ports-${base}-${base + 1}-${base + 2}.lock`);
    fs.mkdirSync(lock);

    const res = run(tree, "e2e-web-session.sh", {
      E2E_PORT_BASE: String(base),
      TMPDIR: tree.root,
    });

    assert.equal(res.status, 1, res.output);
    assert.match(res.output, /another web E2E session is starting or using ports/);
    assert.match(res.output, new RegExp(`Lock: ${lock.replace(/[.*+?^${}()|[\\]\\]/g, "\\$&")}`));
    assert.ok(fs.existsSync(lock), "the failed contender must not remove the active session's lock");
  } finally {
    tree.cleanup();
  }
});

// ── the core reports the port it actually bound ──────────────────────────

test("a core that fell back to another port fails the session", async () => {
  // Two sessions starting at once can both pass the preflight. The mock and the
  // web host then die on EADDRINUSE, but the core survives on a neighbouring
  // port while the probes are answered by whoever holds the requested one.
  const tree = makeTree();
  try {
    const base = await freeBase();
    markBundle(tree, { mockPort: base, corePort: base + 1 });
    stubCore(tree, base + 7, 3_000);

    const res = run(tree, "e2e-web-session.sh", { E2E_PORT_BASE: String(base) });

    assert.equal(res.status, 1, res.output);
    assert.match(res.output, new RegExp(`the core bound 127\\.0\\.0\\.1:${base + 7}`));
    assert.match(res.output, new RegExp(`not 127\\.0\\.0\\.1:${base + 1}`));
  } finally {
    tree.cleanup();
  }
});

test("a core on the requested port runs the specs", async () => {
  // The other half of the check: it must not fail a session whose core bound
  // what it asked for.
  const tree = makeTree();
  try {
    const base = await freeBase();
    markBundle(tree, { mockPort: base, corePort: base + 1 });
    stubCore(tree, base + 1);

    const res = run(tree, "e2e-web-session.sh", { E2E_PORT_BASE: String(base) });

    assert.doesNotMatch(res.output, /the core bound/);
    assert.ok(
      waitForCall(tree, "exec playwright"),
      `expected the specs to run:\n${tree.calls().join("\n")}`,
    );
  } finally {
    tree.cleanup();
  }
});

test("a core without binding evidence fails the session", async () => {
  const tree = makeTree();
  try {
    const base = await freeBase();
    markBundle(tree, { mockPort: base, corePort: base + 1 });
    stubCore(tree, null);

    const res = run(tree, "e2e-web-session.sh", { E2E_PORT_BASE: String(base) });

    assert.equal(res.status, 1, res.output);
    assert.match(res.output, /could not read the core's bound address/);
    assert.match(res.output, /Refusing to continue without binding evidence/);
    assert.doesNotMatch(res.output, /Core RPC authentication failed/);
  } finally {
    tree.cleanup();
  }
});

test("the recorded build ports come from the pre-dotenv selection", async () => {
  // `.env` configures normal development and must not change what the bundle
  // was built for, so the ports recorded next to it are the ones resolved
  // before it was sourced — the same values baked into VITE_BACKEND_URL.
  const tree = makeTree();
  try {
    const base = await freeBase();
    fs.writeFileSync(
      path.join(tree.root, ".env"),
      "E2E_MOCK_PORT=28473\nOPENHUMAN_CORE_PORT=27788\n",
    );

    assert.equal(run(tree, "e2e-web-build.sh", { E2E_PORT_BASE: String(base) }).status, 0);

    const recorded = JSON.parse(
      fs.readFileSync(path.join(tree.root, "app", "dist-web", ".e2e-build-ports.json"), "utf8"),
    );
    assert.equal(recorded.e2e_mock_port, String(base));
    assert.equal(recorded.openhuman_core_port, String(base + 1));
    assert.equal(recorded.vite_backend_url, `http://127.0.0.1:${base}`);
  } finally {
    tree.cleanup();
  }
});

// ── the bundle has to be newer than the sources it was built from ──────────

test("the session refuses a bundle older than the sources", async () => {
  // The defect: the session serves the prebuilt `dist-web`, so a spec re-run
  // after editing `app/src` asserted against the PREVIOUS bundle and passed.
  // A revert-proof or a fault injection then proves nothing (#5919).
  const tree = makeTree();
  try {
    const base = await freeBase();
    markBundle(tree, { mockPort: base, corePort: base + 1 });
    stubCore(tree, base + 1);
    const marker = path.join(tree.root, "app", "dist-web", MARKER);
    const edited = path.join(tree.root, "app", "src", "App.tsx");
    fs.writeFileSync(edited, "// edited\n");
    setMtimeAfter(edited, marker);

    const res = run(tree, "e2e-web-session.sh", { E2E_PORT_BASE: String(base) });

    assert.equal(res.status, 1, res.output);
    assert.match(res.output, /is older than the sources it was built from/);
    assert.match(res.output, /Newer than the bundle: .*app\/src/);
    assert.ok(
      !tree.calls().some((call) => call.includes("exec playwright")),
      `the specs must not run against a stale bundle:\n${tree.calls().join("\n")}`,
    );
  } finally {
    tree.cleanup();
  }
});

test("a file added after the build is caught even when the file itself is old", async () => {
  // Why directories are stat'd and not only files: adding, removing or renaming
  // one need not leave any surviving file newer than the marker, but it does
  // bump the directory's own mtime.
  const tree = makeTree();
  try {
    const base = await freeBase();
    markBundle(tree, { mockPort: base, corePort: base + 1 });
    stubCore(tree, base + 1);
    const added = path.join(tree.root, "app", "src", "Added.tsx");
    fs.writeFileSync(added, "");
    const old = fs.statSync(path.join(tree.root, "app", "index.html")).mtimeMs / 1000;
    fs.utimesSync(added, old, old);
    const marker = path.join(tree.root, "app", "dist-web", MARKER);
    setMtimeAfter(path.join(tree.root, "app", "src"), marker);

    const res = run(tree, "e2e-web-session.sh", { E2E_PORT_BASE: String(base) });

    assert.equal(res.status, 1, res.output);
    assert.match(res.output, /is older than the sources it was built from/);
  } finally {
    tree.cleanup();
  }
});

test("the session runs the specs when the bundle is newer than the sources", async () => {
  // The other half: an untouched checkout must not be refused.
  const tree = makeTree();
  try {
    const base = await freeBase();
    markBundle(tree, { mockPort: base, corePort: base + 1 });
    stubCore(tree, base + 1);

    const res = run(tree, "e2e-web-session.sh", { E2E_PORT_BASE: String(base) });

    assert.doesNotMatch(res.output, /older than the sources/);
    assert.ok(
      waitForCall(tree, "exec playwright"),
      `expected the specs to run:\n${tree.calls().join("\n")}`,
    );
  } finally {
    tree.cleanup();
  }
});

test("a bundle input that moved fails the check instead of disabling it", async () => {
  // `find` on a path that does not exist reports nothing, so a renamed input
  // would silently turn the whole gate off and serve the stale bundle again.
  const tree = makeTree();
  try {
    const base = await freeBase();
    markBundle(tree, { mockPort: base, corePort: base + 1 });
    stubCore(tree, base + 1);
    fs.rmSync(path.join(tree.root, "app", "vite.config.ts"));

    const res = run(tree, "e2e-web-session.sh", { E2E_PORT_BASE: String(base) });

    assert.equal(res.status, 1, res.output);
    assert.match(res.output, /bundle input .*vite\.config\.ts does not exist/);
    assert.match(res.output, /Update the input list/);
  } finally {
    tree.cleanup();
  }
});

// ── the lane that owns these tests has to run for them ─────────────────────

test("a change to either web E2E script arms the lane that runs this suite", () => {
  // These scripts match the frontend filter, which runs no node --test suite.
  // Without an entry in the `scripts` filter, editing one of them would skip
  // the tests above and the bundle guard's — the gate would stop watching
  // itself, which is the same trap the filter file already documents.
  const filter = fs.readFileSync(
    path.join(repoRoot, ".github", "ci-paths-filter.yml"),
    "utf8",
  );
  const header = "\nscripts:\n";
  const start = filter.indexOf(header);
  assert.ok(start !== -1, "no `scripts:` filter in .github/ci-paths-filter.yml");
  // Up to the next top-level key, so a path listed under another filter does
  // not make this pass.
  const scriptsBlock = filter.slice(start + header.length).split(/\n(?=\S)/)[0];
  assert.ok(scriptsBlock.trim().length > 0, "the `scripts:` filter parsed empty");
  for (const script of APP_SCRIPTS) {
    assert.ok(
      scriptsBlock.includes(`app/scripts/${script}`),
      `app/scripts/${script} must be in the scripts paths filter`,
    );
  }
});

// ── the specs have to be told the port too ─────────────────────────────────

test("the session exports E2E_MOCK_PORT for the Playwright specs", () => {
  // Every spec builds its own `MOCK_ADMIN_BASE` from this variable and falls
  // back to 18473, so a session on a derived block used to drive the wrong
  // mock's admin API while the core talked to the right one.
  const source = fs.readFileSync(
    path.join(repoRoot, "app", "scripts", "e2e-web-session.sh"),
    "utf8",
  );
  assert.match(source, /^export E2E_MOCK_PORT$/m);

  const specs = path.join(repoRoot, "app", "test", "playwright", "specs");
  const readers = fs
    .readdirSync(specs)
    .filter((f) => f.endsWith(".spec.ts"))
    .filter((f) => fs.readFileSync(path.join(specs, f), "utf8").includes("E2E_MOCK_PORT"));
  assert.ok(readers.length > 0, "expected Playwright specs to read E2E_MOCK_PORT");
});
