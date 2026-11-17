// Pure helpers for `pnpm debug web` (scripts/debug/web-ui.mjs), split out so
// `scripts/__tests__/debug-web-ui.test.mjs` can cover them without booting a
// browser, a core or Vite.
import net from "node:net";
import path from "node:path";

/** Merge child overrides into an inherited environment; null explicitly unsets a key. */
export function mergeChildEnv(parent, overrides) {
  const env = { ...parent };
  for (const [key, value] of Object.entries(overrides)) {
    if (value === null) delete env[key];
    else env[key] = value;
  }
  return env;
}

/** Isolated environment and working directory for the throwaway core process. */
export function coreLaunchSpec(
  parentEnv,
  { workspace, mockUrl, token, logLevel = parentEnv.RUST_LOG ?? "info" },
) {
  return {
    cwd: workspace,
    env: {
      OPENHUMAN_WORKSPACE: workspace,
      OPENHUMAN_DOTENV_PATH: path.join(workspace, "empty.env"),
      OPENHUMAN_CORE_TOKEN: token,
      BACKEND_URL: mockUrl,
      VITE_BACKEND_URL: mockUrl,
      OPENHUMAN_COMPOSIO_MODE: "disabled",
      OPENHUMAN_SANDBOX: "off",
      OPENHUMAN_BACKEND_API_KEY: null,
      OPENHUMAN_BACKEND_SESSION_TOKEN: null,
      OPENHUMAN_STORAGE_URL: null,
      RUST_LOG: logLevel,
    },
  };
}

/** Pass the isolated launch specification through to the child process launcher. */
export function startCoreChild(startChild, core, corePort, launchSpec) {
  return startChild(
    "core",
    core,
    ["serve", "--port", String(corePort)],
    launchSpec.env,
    launchSpec.cwd,
  );
}

/** Whether a logged core RPC auth state reports an authenticated credential. */
export function isCoreAuthenticated(authState) {
  return authState?.result?.isAuthenticated === true;
}

/** Navigate an authenticated scripted session to chat when Home has no composer. */
export async function ensureSignedInChat(page, appOrigin) {
  const composer = page.getByRole("textbox", { name: "Message input" });
  if (!(await composer.isVisible().catch(() => false))) {
    await page.goto(`${appOrigin}/#/chat`);
  }
  await composer.waitFor({ state: "visible", timeout: 120_000 });
}

/** Create an authenticated core RPC client with a bounded request lifetime. */
export function createCoreRpcClient(
  rpcUrl,
  token,
  fetchImpl = fetch,
  timeoutMs = 10_000,
) {
  return async (method, params = {}) => {
    const res = await fetchImpl(rpcUrl, {
      method: "POST",
      headers: {
        "content-type": "application/json",
        authorization: `Bearer ${token}`,
      },
      body: JSON.stringify({ jsonrpc: "2.0", id: Date.now(), method, params }),
      signal: AbortSignal.timeout(timeoutMs),
    });
    const body = await res.json();
    if (body.error) throw new Error(`${method}: ${body.error.message}`);
    return body.result;
  };
}

export const USAGE = `Usage: pnpm debug web [options]

Boots a throwaway stack — mock backend, a fresh \`openhuman-core serve\` on a
scratch workspace, and Vite without a file watcher — then opens the SPA in
Playwright Chromium and signs in through the real provider button.

Options:
  --script <file.mjs>  Run a script against the signed-in page, then exit. Its
                       default export receives { page, context, browser, mock,
                       rpc, urls, logDir, screenshot, log }.
  --headed             Show the browser window (needs the full Chromium build).
  --keep               Keep the stack up after --script finishes (Ctrl-C ends it).
  --core <path>        Core binary (default: target/debug/openhuman-core).
  --no-build           Do not build the core binary when it is missing.
  --no-sign-in         Stop on the welcome page instead of signing in.
  -h, --help           Show this help.

Without --script the stack stays up and prints its URLs until Ctrl-C.
Artifacts (core/vite/browser logs, screenshots) go to
target/debug-logs/web-<timestamp>/.`;

/** Parse argv into options. Throws on an unknown flag or a missing value. */
export function parseArgs(argv) {
  const opts = {
    script: null,
    headed: false,
    keep: false,
    core: null,
    build: true,
    signIn: true,
    help: false,
  };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    const value = () => {
      const next = argv[i + 1];
      if (next === undefined || next.startsWith("--")) {
        throw new Error(`${arg} requires a value`);
      }
      i += 1;
      return next;
    };
    switch (arg) {
      case "--script":
        opts.script = value();
        break;
      case "--core":
        opts.core = value();
        break;
      case "--headed":
        opts.headed = true;
        break;
      case "--keep":
        opts.keep = true;
        break;
      case "--no-build":
        opts.build = false;
        break;
      case "--no-sign-in":
        opts.signIn = false;
        break;
      case "-h":
      case "--help":
        opts.help = true;
        break;
      default:
        throw new Error(`unknown option: ${arg}`);
    }
  }
  return opts;
}

/** A free loopback TCP port, chosen by the kernel. */
export function freePort() {
  return new Promise((resolve, reject) => {
    const srv = net.createServer();
    srv.unref();
    srv.on("error", reject);
    srv.listen(0, "127.0.0.1", () => {
      const { port } = srv.address();
      srv.close(() => resolve(port));
    });
  });
}

/** `YYYYMMDD-HHMMSS` in local time, for the per-run artifact directory. */
export function runStamp(date = new Date()) {
  const pad = (n) => String(n).padStart(2, "0");
  return (
    `${date.getFullYear()}${pad(date.getMonth() + 1)}${pad(date.getDate())}-` +
    `${pad(date.getHours())}${pad(date.getMinutes())}${pad(date.getSeconds())}`
  );
}

/**
 * The `/__dev-connect` URL that seeds the SPA's core RPC URL and bearer into
 * localStorage (see `devConnectPlugin` in app/vite.config.ts).
 */
export function devConnectUrl(appOrigin, rpcUrl, token) {
  const fragment = new URLSearchParams({ rpcUrl, token });
  return `${appOrigin}/__dev-connect#${fragment}`;
}

/** Poll `fn` until it resolves truthy, or throw `what` after `timeoutMs`. */
export async function waitFor(fn, { timeoutMs, intervalMs = 200, what }) {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    try {
      if (await fn()) return;
    } catch {
      // not ready yet
    }
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${what}`);
    await new Promise((resolve) => setTimeout(resolve, intervalMs));
  }
}

const LOOPBACK_HOSTS = new Set(["localhost", "127.0.0.1", "[::1]"]);

/** True for http(s)/ws(s) URLs on a loopback host, plus data:/blob: URLs. */
export function isLoopback(url) {
  const parsed = typeof url === "string" ? new URL(url) : url;
  if (parsed.protocol === "data:" || parsed.protocol === "blob:") return true;
  return LOOPBACK_HOSTS.has(parsed.hostname);
}
