#!/usr/bin/env node
// `pnpm debug web` — drive the web SPA in Playwright Chromium against a
// throwaway, fully local stack: the shared mock backend (in-process), a fresh
// `openhuman-core serve` on a scratch workspace, and Vite with no file watcher.
// See USAGE in ./web-ui-lib.mjs.
//
// Why each piece is set up the way it is:
// - Vite runs with OPENHUMAN_VITE_NO_WATCH=1. A busy box can have its inotify
//   watch limit used up by other processes, and the watcher's ENOSPC is fatal
//   to the dev server; a scripted browser needs no HMR anyway.
// - The SPA is pointed at the core through Vite's `/__dev-connect` page, which
//   seeds the RPC URL and bearer into localStorage.
// - Sign-in goes through the real provider button: the mock answers
//   `/auth/<provider>/login?redirectUri=<vite>/__dev-auth` like the backend,
//   so the app's own OAuth callback path runs.
import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import process from "node:process";
import { fileURLToPath, pathToFileURL } from "node:url";

import {
  devConnectUrl,
  ensureSignedInChat,
  coreLaunchSpec,
  createCoreRpcClient,
  isCoreAuthenticated,
  freePort,
  isLoopback,
  mergeChildEnv,
  parseArgs,
  runStamp,
  startCoreChild,
  USAGE,
  waitFor,
} from "./web-ui-lib.mjs";

const repo = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../..",
);
const log = (...args) => console.log("[debug:web]", ...args);

let opts;
try {
  opts = parseArgs(process.argv.slice(2));
} catch (err) {
  console.error(`[debug:web] ${err.message}\n\n${USAGE}`);
  process.exit(2);
}
if (opts.help) {
  console.log(USAGE);
  process.exit(0);
}

const logDir = path.join(repo, "target/debug-logs", `web-${runStamp()}`);
fs.mkdirSync(logDir, { recursive: true });
const children = [];
let mockModule = null;
let browser = null;
let shuttingDown = false;

async function shutdown(code) {
  if (shuttingDown) return;
  shuttingDown = true;
  log(`shutting down (exit ${code})`);
  await browser?.close().catch(() => {});
  for (const child of children) {
    // Each child leads its own process group, so pnpm/node grandchildren go too.
    try {
      process.kill(-child.pid, "SIGTERM");
      log(`sent SIGTERM to ${child.name} group ${child.pid}`);
    } catch (err) {
      log(
        `group kill of ${child.name} failed (${err.code}); signalling the pid`,
      );
      child.kill("SIGTERM");
    }
  }
  await mockModule?.stopMockServer().catch(() => {});
  log(`artifacts: ${path.relative(repo, logDir)}`);
  process.exit(code);
}
process.on("SIGINT", () => void shutdown(130));
process.on("SIGTERM", () => void shutdown(143));

function startChild(name, command, args, env, cwd = repo) {
  const out = fs.openSync(path.join(logDir, `${name}.log`), "a");
  // Debug CLI futures can exceed macOS's 8 MiB main-thread stack. Give the
  // isolated test core the same headroom as the desktop runtime's workers.
  // Keep paths/arguments as positional parameters, never interpolated shell code.
  const largeStack = name === "core" && process.platform === "darwin";
  const launchCommand = largeStack ? "/bin/sh" : command;
  const launchArgs = largeStack
    ? [
        "-c",
        'ulimit -s 65520 || exit 1; exec "$@"',
        "debug-web-core",
        command,
        ...args,
      ]
    : args;
  const child = spawn(launchCommand, launchArgs, {
    cwd,
    env: mergeChildEnv(process.env, env),
    stdio: ["ignore", out, out],
    detached: true,
  });
  child.on("exit", (status) => {
    if (!shuttingDown) {
      console.error(`[debug:web] ${name} exited (${status}); see ${name}.log`);
      void shutdown(1);
    }
  });
  child.name = name;
  children.push(child);
  return child;
}

function resolveCore() {
  const core = path.resolve(repo, opts.core ?? "target/debug/openhuman-core");
  if (fs.existsSync(core)) return core;
  if (opts.core || !opts.build) {
    throw new Error(`core binary not found: ${core}`);
  }
  log("building openhuman-core (target/debug/openhuman-core is missing)…");
  const build = spawnSync(
    "cargo",
    [
      "build",
      "--manifest-path",
      "Cargo.toml",
      "-p",
      "openhuman-cli",
      "--bin",
      "openhuman-core",
    ],
    { cwd: repo, stdio: "inherit" },
  );
  if (build.status !== 0) throw new Error("cargo build failed");
  return core;
}

function loadChromium() {
  const appRequire = createRequire(path.join(repo, "app/package.json"));
  const { chromium } = appRequire("@playwright/test");
  if (opts.headed && !fs.existsSync(chromium.executablePath())) {
    throw new Error(
      `headed mode needs the full Chromium build at ${chromium.executablePath()}.\n` +
        "Install it with: pnpm --filter openhuman-app exec playwright install chromium",
    );
  }
  return chromium;
}

async function signIn(page, rpc, appOrigin) {
  await page.getByText("Welcome to OpenHuman").waitFor({ timeout: 120_000 });
  log("signing in through the GitHub provider button (mock backend)…");
  await page.getByRole("button", { name: "GitHub" }).click();
  await waitFor(
    async () => isCoreAuthenticated(await rpc("openhuman.auth_get_state")),
    {
      timeoutMs: 120_000,
      what: "the core's authenticated state after mock sign-in",
    },
  );
  await ensureSignedInChat(page, appOrigin);
}

async function main() {
  const core = resolveCore();
  const chromium = loadChromium();
  const [mockPort, corePort, vitePort] = await Promise.all([
    freePort(),
    freePort(),
    freePort(),
  ]);
  const token = `debug-web-${Math.random().toString(36).slice(2)}${Date.now()}`;
  const mockUrl = `http://127.0.0.1:${mockPort}`;
  const rpcUrl = `http://127.0.0.1:${corePort}/rpc`;
  const appOrigin = `http://localhost:${vitePort}`;
  const workspace = path.join(logDir, "workspace");
  fs.mkdirSync(workspace, { recursive: true });
  const emptyDotenv = path.join(workspace, "empty.env");
  fs.writeFileSync(emptyDotenv, "");

  mockModule = await import(
    pathToFileURL(path.join(repo, "scripts/mock-api-core.mjs"))
  );
  await mockModule.startMockServer(mockPort);
  log(`mock backend   ${mockUrl}`);

  const coreLaunch = coreLaunchSpec(process.env, {
    workspace,
    mockUrl,
    token,
    logLevel: process.env.RUST_LOG ?? "info",
  });
  startCoreChild(startChild, core, corePort, coreLaunch);
  await waitFor(
    async () => (await fetch(`http://127.0.0.1:${corePort}/health`)).ok,
    {
      timeoutMs: 120_000,
      what: "the core's /health",
    },
  );
  log(`core           ${rpcUrl} (workspace ${path.relative(repo, workspace)})`);

  startChild(
    "vite",
    path.join(repo, "app/node_modules/.bin/vite"),
    ["--port", String(vitePort), "--strictPort"],
    {
      OPENHUMAN_VITE_NO_WATCH: "1",
      OPENHUMAN_DEV_PORT: String(vitePort),
      OPENHUMAN_CORE_TOKEN: token,
      VITE_OPENHUMAN_CORE_RPC_URL: rpcUrl,
      VITE_BACKEND_URL: mockUrl,
      VITE_DEV_SKIP_ONBOARDING: "true",
    },
    path.join(repo, "app"),
  );
  await waitFor(async () => (await fetch(appOrigin)).ok, {
    timeoutMs: 120_000,
    what: "Vite",
  });
  log(`app            ${appOrigin}`);

  browser = await chromium.launch({
    headless: !opts.headed,
    args: ["--disable-dev-shm-usage"],
    // Playwright's own signal handlers close the browser and exit the
    // process, which would skip `shutdown` and leak the core and Vite.
    handleSIGINT: false,
    handleSIGTERM: false,
    handleSIGHUP: false,
  });
  const context = await browser.newContext({
    viewport: { width: 1300, height: 900 },
  });
  const browserLog = fs.createWriteStream(path.join(logDir, "browser.log"));
  // Everything the run needs is on loopback. Abort the rest (analytics,
  // remote assets) so a scenario never reaches a real third-party service.
  await context.route(
    (url) => !isLoopback(url),
    (route) => {
      browserLog.write(
        `[blocked] ${route.request().method()} ${route.request().url()}\n`,
      );
      return route.abort("blockedbyclient");
    },
  );
  const page = await context.newPage();
  page.on("console", (msg) =>
    browserLog.write(`[${msg.type()}] ${msg.text()}\n`),
  );
  page.on("pageerror", (err) =>
    browserLog.write(`[pageerror] ${err.stack ?? err.message}\n`),
  );
  page.on("requestfailed", (req) =>
    browserLog.write(
      `[requestfailed] ${req.method()} ${req.url()} ${req.failure()?.errorText}\n`,
    ),
  );
  page.on("crash", () => browserLog.write("[crash] renderer crashed\n"));

  const screenshot = async (name) => {
    const file = path.join(logDir, `${name}.png`);
    await page.screenshot({ path: file, fullPage: true });
    return file;
  };

  const rpc = createCoreRpcClient(rpcUrl, token);

  // The first load compiles the app's modules on demand; give it room.
  page.setDefaultNavigationTimeout(120_000);
  await page.goto(devConnectUrl(appOrigin, rpcUrl, token));
  if (opts.signIn) {
    await signIn(page, rpc, appOrigin);
    log(`signed in      ${page.url()}`);
  }
  await screenshot("ready");

  const mock = {
    url: mockUrl,
    set: (key, value) =>
      mockModule.setMockBehavior(
        key,
        typeof value === "string" ? value : JSON.stringify(value),
      ),
    reset: () => mockModule.resetMockBehavior(),
    requests: () => mockModule.getRequestLog(),
  };
  const urls = { app: appOrigin, rpc: rpcUrl, mock: mockUrl, token };

  if (!opts.script) {
    log("ready — the stack stays up until Ctrl-C");
    log(
      `open ${devConnectUrl(appOrigin, rpcUrl, token)} in any local browser to share this core`,
    );
    await new Promise(() => {});
  }

  const scriptPath = path.resolve(process.cwd(), opts.script);
  log(`running ${path.relative(repo, scriptPath)}`);
  const { default: run } = await import(pathToFileURL(scriptPath));
  if (typeof run !== "function")
    throw new Error("--script must default-export a function");
  try {
    await run({
      page,
      context,
      browser,
      mock,
      rpc,
      urls,
      logDir,
      screenshot,
      log,
    });
  } catch (err) {
    await screenshot("failure").catch(() => {});
    throw err;
  }
  log("script finished");
  if (opts.keep) {
    log("--keep: the stack stays up until Ctrl-C");
    await new Promise(() => {});
  }
}

main().then(
  () => shutdown(0),
  async (err) => {
    console.error(`[debug:web] ${err.stack ?? err.message}`);
    await shutdown(1);
  },
);
