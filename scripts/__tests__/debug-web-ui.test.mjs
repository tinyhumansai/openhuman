import assert from "node:assert/strict";
import net from "node:net";
import test from "node:test";

import {
  coreLaunchSpec,
  createCoreRpcClient,
  devConnectUrl,
  ensureSignedInChat,
  mergeChildEnv,
  freePort,
  isCoreAuthenticated,
  isLoopback,
  parseArgs,
  runStamp,
  startCoreChild,
  waitFor,
} from "../debug/web-ui-lib.mjs";

test("isCoreAuthenticated reads the logged auth result envelope", () => {
  assert.equal(
    isCoreAuthenticated({
      logs: ["session state fetched"],
      result: { isAuthenticated: true },
    }),
    true,
  );
  assert.equal(
    isCoreAuthenticated({
      logs: ["session state fetched"],
      result: { isAuthenticated: false },
    }),
    false,
  );
});

test("ensureSignedInChat opens the canonical chat route from an authenticated home", async () => {
  let visible = false;
  let navigatedTo;
  const page = {
    getByRole(role, options) {
      assert.equal(role, "textbox");
      assert.equal(options.name, "Message input");
      return {
        isVisible: async () => visible,
        waitFor: async ({ state }) => {
          assert.equal(state, "visible");
          assert.equal(visible, true);
        },
      };
    },
    async goto(url) {
      navigatedTo = url;
      visible = true;
    },
  };

  await ensureSignedInChat(page, "http://localhost:1420");
  assert.equal(navigatedTo, "http://localhost:1420/#/chat");
});

test("ensureSignedInChat keeps an existing composer on its current route", async () => {
  let waited = false;
  const page = {
    getByRole: () => ({
      isVisible: async () => true,
      waitFor: async ({ state, timeout }) => {
        assert.equal(state, "visible");
        assert.equal(timeout, 120_000);
        waited = true;
      },
    }),
    goto: async () =>
      assert.fail("a visible chat composer should not navigate"),
  };

  await ensureSignedInChat(page, "http://localhost:1420");
  assert.equal(waited, true);
});

test("ensureSignedInChat propagates its bounded composer wait failure", async () => {
  const timeout = new Error("composer did not appear");
  let navigatedTo;
  const page = {
    getByRole: () => ({
      isVisible: async () => false,
      waitFor: async ({ state, timeout: timeoutMs }) => {
        assert.equal(state, "visible");
        assert.equal(timeoutMs, 120_000);
        throw timeout;
      },
    }),
    goto: async (url) => {
      navigatedTo = url;
    },
  };

  await assert.rejects(
    ensureSignedInChat(page, "http://localhost:1420"),
    (error) => error === timeout,
  );
  assert.equal(navigatedTo, "http://localhost:1420/#/chat");
});

test("core RPC client bounds a stalled request and propagates its timeout", async () => {
  let signal;
  const hangingFetch = (_url, init) => {
    signal = init.signal;
    return new Promise((_resolve, reject) => {
      signal.addEventListener("abort", () => reject(signal.reason), {
        once: true,
      });
    });
  };
  const rpc = createCoreRpcClient(
    "http://127.0.0.1:7788/rpc",
    "test-token",
    hangingFetch,
    5,
  );
  // AbortSignal.timeout uses an unreferenced timer; keep the test process alive
  // until that timeout fires even when no other handles are open.
  const keepAlive = setTimeout(() => {}, 50);
  try {
    await assert.rejects(rpc("openhuman.auth_get_state"), {
      name: "TimeoutError",
    });
  } finally {
    clearTimeout(keepAlive);
  }
  assert.equal(signal.aborted, true);
});

test("coreLaunchSpec isolates the core from operator credentials, storage, and dotenv", () => {
  const inheritedEnv = {
    OPENHUMAN_BACKEND_API_KEY: "inherited",
    OPENHUMAN_BACKEND_SESSION_TOKEN: "inherited",
    OPENHUMAN_STORAGE_URL: "inherited",
    OPENHUMAN_DOTENV_PATH: "/operator/.env",
    RUST_LOG: "trace",
  };
  const spec = coreLaunchSpec(inheritedEnv, {
    repo: "/checkout/openhuman",
    workspace: "/private/run/workspace",
    mockUrl: "http://127.0.0.1:18500",
    token: "test-core-token",
    logLevel: "info",
  });
  const finalChildEnv = mergeChildEnv(inheritedEnv, spec.env);
  assert.deepEqual(
    {
      cwd: spec.cwd,
      workspace: finalChildEnv.OPENHUMAN_WORKSPACE,
      dotenv: finalChildEnv.OPENHUMAN_DOTENV_PATH,
      backend: finalChildEnv.BACKEND_URL,
      viteBackend: finalChildEnv.VITE_BACKEND_URL,
      token: finalChildEnv.OPENHUMAN_CORE_TOKEN,
      apiKey: finalChildEnv.OPENHUMAN_BACKEND_API_KEY,
      sessionToken: finalChildEnv.OPENHUMAN_BACKEND_SESSION_TOKEN,
      storageUrl: finalChildEnv.OPENHUMAN_STORAGE_URL,
      logLevel: finalChildEnv.RUST_LOG,
    },
    {
      cwd: "/private/run/workspace",
      workspace: "/private/run/workspace",
      dotenv: "/private/run/workspace/empty.env",
      backend: "http://127.0.0.1:18500",
      viteBackend: "http://127.0.0.1:18500",
      token: "test-core-token",
      apiKey: undefined,
      sessionToken: undefined,
      storageUrl: undefined,
      logLevel: "info",
    },
  );

  let launch;
  startCoreChild(
    (...args) => (launch = args),
    "/private/openhuman-core",
    18473,
    spec,
  );
  assert.equal(launch[0], "core");
  assert.equal(launch[1], "/private/openhuman-core");
  assert.deepEqual(launch[2], ["serve", "--port", "18473"]);
  assert.equal(launch[3], spec.env);
  assert.equal(launch[4], spec.cwd);
  const finalChildEnvFromActualLaunch = mergeChildEnv(inheritedEnv, launch[3]);
  assert.equal(
    finalChildEnvFromActualLaunch.OPENHUMAN_BACKEND_API_KEY,
    undefined,
  );
  assert.equal(
    finalChildEnvFromActualLaunch.OPENHUMAN_BACKEND_SESSION_TOKEN,
    undefined,
  );
  assert.equal(finalChildEnvFromActualLaunch.OPENHUMAN_STORAGE_URL, undefined);
  assert.equal(
    finalChildEnvFromActualLaunch.OPENHUMAN_DOTENV_PATH,
    "/private/run/workspace/empty.env",
  );
});

test("mergeChildEnv overrides and removes inherited variables", () => {
  assert.deepEqual(
    mergeChildEnv(
      { KEEP: "parent", REMOVE: "credential", OVERRIDE: "parent" },
      { REMOVE: null, OVERRIDE: "private", ADD: "value" },
    ),
    { KEEP: "parent", OVERRIDE: "private", ADD: "value" },
  );
});

test("parseArgs defaults to an interactive, signed-in run that builds the core", () => {
  assert.deepEqual(parseArgs([]), {
    script: null,
    headed: false,
    keep: false,
    core: null,
    build: true,
    signIn: true,
    help: false,
  });
});

test("parseArgs reads every flag", () => {
  const opts = parseArgs([
    "--script",
    "s.mjs",
    "--core",
    "bin/core",
    "--headed",
    "--keep",
    "--no-build",
    "--no-sign-in",
  ]);
  assert.equal(opts.script, "s.mjs");
  assert.equal(opts.core, "bin/core");
  assert.equal(opts.headed, true);
  assert.equal(opts.keep, true);
  assert.equal(opts.build, false);
  assert.equal(opts.signIn, false);
  assert.equal(parseArgs(["-h"]).help, true);
});

test("parseArgs rejects unknown flags and missing values", () => {
  assert.throws(() => parseArgs(["--bogus"]), /unknown option: --bogus/);
  assert.throws(() => parseArgs(["--script"]), /--script requires a value/);
  assert.throws(
    () => parseArgs(["--core", "--keep"]),
    /--core requires a value/,
  );
});

test("devConnectUrl carries the RPC URL and bearer in the fragment", () => {
  const url = new URL(
    devConnectUrl("http://localhost:1420", "http://127.0.0.1:7788/rpc", "tok"),
  );
  assert.equal(url.pathname, "/__dev-connect");
  assert.equal(url.search, "");
  const fragment = new URLSearchParams(url.hash.slice(1));
  assert.equal(fragment.get("rpcUrl"), "http://127.0.0.1:7788/rpc");
  assert.equal(fragment.get("token"), "tok");
});

test("runStamp is a sortable local timestamp", () => {
  assert.equal(runStamp(new Date(2026, 9, 7, 9, 5, 3)), "20261007-090503");
});

test("freePort returns a port that can be bound", async () => {
  const port = await freePort();
  await new Promise((resolve, reject) => {
    const srv = net
      .createServer()
      .listen(port, "127.0.0.1", () => srv.close(resolve));
    srv.on("error", reject);
  });
});

test("waitFor resolves once the probe passes and times out otherwise", async () => {
  let calls = 0;
  await waitFor(async () => ++calls >= 3, {
    timeoutMs: 1000,
    intervalMs: 1,
    what: "x",
  });
  assert.equal(calls, 3);
  await assert.rejects(
    waitFor(async () => false, {
      timeoutMs: 20,
      intervalMs: 5,
      what: "the thing",
    }),
    /timed out waiting for the thing/,
  );
});

test("isLoopback admits only local hosts and inline URLs", () => {
  assert.equal(isLoopback("http://localhost:1420/src/main.tsx"), true);
  assert.equal(isLoopback("http://127.0.0.1:7788/rpc"), true);
  assert.equal(isLoopback("ws://127.0.0.1:7788/socket.io/"), true);
  assert.equal(isLoopback("data:image/png;base64,AAAA"), true);
  assert.equal(isLoopback("https://panel.tinyhumans.ai/api/track"), false);
  assert.equal(isLoopback("https://raw.githubusercontent.com/x"), false);
});
