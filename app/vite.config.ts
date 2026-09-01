import { defineConfig, type PluginOption } from "vite";
import react from "@vitejs/plugin-react";
import { sentryVitePlugin } from "@sentry/vite-plugin";

import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { nodePolyfills } from "vite-plugin-node-polyfills";

const host = process.env.TAURI_DEV_HOST;

// Optional override so parallel `dev:app:win` runs across worktrees can
// avoid the hardcoded 1420 collision. Default 1420 preserves prior behavior;
// HMR companion port is dev port + 1 (used only when TAURI_DEV_HOST is set).
const devPort = Number(process.env.OPENHUMAN_DEV_PORT) || 1420;
const hmrPort = devPort + 1;

const __dirname = dirname(fileURLToPath(import.meta.url));
const pkg = JSON.parse(
  readFileSync(resolve(__dirname, "package.json"), "utf8"),
) as { version: string };

// Canonical Sentry release — must stay in sync with the string produced by
// `SENTRY_RELEASE` in app/src/utils/config.ts and the core sidecar's
// `sentry::init` in src/main.rs so events from every surface group together.
function computeSentryRelease(): string {
  const raw = (process.env.SENTRY_RELEASE ?? "").trim();
  if (raw) return raw;
  const sha = (process.env.VITE_BUILD_SHA ?? "").trim().slice(0, 12);
  return sha
    ? `openhuman@${pkg.version}+${sha}`
    : `openhuman@${pkg.version}`;
}

// Gate source-map upload on the presence of SENTRY_AUTH_TOKEN so local dev
// and CI jobs that don't ship to users skip the plugin silently. The
// companion `SENTRY_ORG` / `SENTRY_PROJECT` come from CI env.
function maybeSentryPlugin(): PluginOption | null {
  const authToken = process.env.SENTRY_AUTH_TOKEN;
  if (!authToken) return null;
  return sentryVitePlugin({
    authToken,
    // Self-hosted Sentry — without `url`, the plugin defaults to sentry.io
    // and silently no-ops the upload. Falls back to sentry.io if unset for
    // local builds against the SaaS instance.
    url: process.env.SENTRY_URL,
    org: process.env.SENTRY_ORG,
    project: process.env.SENTRY_PROJECT,
    release: {
      name: computeSentryRelease(),
      // The frontend already passes this release to Sentry.init(). Keeping the
      // plugin's virtual release module enabled can be transformed by the node
      // polyfill injector into startup code that calls Rollup helpers before
      // they are initialized in the generated desktop bundle.
      inject: false,
    },
    sourcemaps: {
      // Vite emits hashed asset files into `app/dist/assets/`. Upload every
      // .js / .map the build produces.
      //
      // `assets` is resolved by sentry-vite-plugin against `process.cwd()`,
      // not the Vite `root` — so a relative path like `../dist/**` would
      // miss when `pnpm tauri build` runs with cwd=`app/` and silently emit
      // `Didn't find any matching sources for debug ID upload`. Use absolute
      // paths anchored at this config file's directory (`app/`) to be
      // immune to whatever cwd the parent process sets.
      assets: [
        resolve(__dirname, "dist/**/*.js"),
        resolve(__dirname, "dist/**/*.map"),
      ],
      // Never ship raw .map files to end users; the upload keeps a copy
      // server-side for symbolication while the bundled app strips them.
      filesToDeleteAfterUpload: [resolve(__dirname, "dist/**/*.map")],
    },
    // Release tagging + commits are handled by sentry-cli / the plugin
    // itself when AUTH_TOKEN and CI env (GITHUB_SHA etc.) are present.
    telemetry: false,
  });
}

function guardCefRelListSupportsPlugin(): PluginOption {
  return {
    name: "openhuman:guard-cef-rel-list-supports",
    enforce: "post",
    renderChunk(code) {
      const unsafe =
        'relList && relList.supports && relList.supports("modulepreload")';
      const guarded =
        'relList && typeof relList.supports === "function" && relList.supports("modulepreload")';
      const next = code.split(unsafe).join(guarded);
      return next === code ? null : { code: next, map: null };
    },
  };
}

// Dev-server-only route that hands the browser the credentials it needs to
// talk to a local `openhuman-core` before the SPA boots.
//
// In the desktop app the shell injects the per-launch bearer over IPC
// (`core_rpc_token`), so the renderer never needs one on disk. A plain browser
// has no such channel: `coreRpcClient` falls back to `getStoredCoreToken()` /
// `peekStoredRpcUrl()`, which read `localStorage`. Without seeding those the
// first RPC is a 401 and the app is stuck on the boot gate.
//
// `apply: "serve"` keeps this out of every production bundle — the route only
// exists while the dev server is running. The token is read from the dev
// server's own environment and never accepted from the query string, so a
// stray link cannot point a running dev session at an attacker's core.
//
// `server.host` defaults to `true` above (binds every adapter, not just
// loopback) to work around the Windows dual-stack `localhost` proxy issue,
// so this route is reachable from the LAN even though it needs no auth of
// its own. Restrict it to loopback callers: a LAN peer that can reach this
// port must not be able to read `OPENHUMAN_CORE_TOKEN` back out of it,
// especially when combined with the documented `OPENHUMAN_CORE_HOST=0.0.0.0`
// setup, where that token would then work against the core directly.
//
// Keys must stay in sync with `app/src/utils/configPersistence.ts`.
function isLoopbackAddress(address: string | undefined): boolean {
  if (!address) return false;
  // Strip an IPv4-mapped-IPv6 prefix (`::ffff:127.0.0.1`) before comparing.
  const normalized = address.replace(/^::ffff:/, "");
  return normalized === "127.0.0.1" || normalized === "::1";
}

function devConnectPlugin(): PluginOption {
  return {
    name: "openhuman:dev-connect",
    apply: "serve",
    configureServer(server) {
      server.middlewares.use("/__dev-connect", (req, res) => {
        if (!isLoopbackAddress(req.socket.remoteAddress)) {
          res.statusCode = 403;
          res.setHeader("Content-Type", "text/plain; charset=utf-8");
          res.end("Forbidden: /__dev-connect is only reachable from loopback.");
          return;
        }

        const token = (process.env.OPENHUMAN_CORE_TOKEN ?? "").trim();
        const rpcUrl = (
          process.env.VITE_OPENHUMAN_CORE_RPC_URL ?? ""
        ).trim();

        // These are the same three keys BootCheckGate's picker writes on a
        // cloud-mode confirm. The mode marker matters: without it
        // `getStoredCoreMode()` returns null, which reads as "the picker has
        // not run yet", and the gate parks the app on the Connect-to-Your-
        // Runtime screen. A local core reached over an explicit URL + bearer
        // is exactly what "cloud" means to that picker.
        //
        // `</script>` inside a JSON string would close the block early.
        const json = (value: string) =>
          JSON.stringify(value).replace(/</g, "\\u003c");

        const html = `<!doctype html>
<html>
  <head><meta charset="utf-8" /><title>Connecting…</title></head>
  <body style="font:14px system-ui;padding:2rem">
    <p>Connecting to local openhuman-core…</p>
    <script>
      try {
        var url = ${json(rpcUrl)};
        var token = ${json(token)};
        if (url) localStorage.setItem("openhuman_core_rpc_url", url);
        if (token) localStorage.setItem("openhuman_core_rpc_token", token);
        if (url && token) localStorage.setItem("openhuman_core_mode", "cloud");
      } catch (err) {
        document.body.textContent =
          "localStorage unavailable: " + err + " — cannot seed core credentials.";
        throw err;
      }
      location.replace("/");
    </script>
  </body>
</html>`;

        res.setHeader("Content-Type", "text/html; charset=utf-8");
        res.setHeader("Cache-Control", "no-store");
        res.end(html);
      });
    },
  };
}

// `VITE_OPENHUMAN_TARGET=web` switches the build to the browser-hosted
// flavor: output lands in `dist-web/` so the desktop build artifact in
// `dist/` (consumed by `cargo tauri build`) is never clobbered, and the
// `import.meta.env.VITE_OPENHUMAN_TARGET` value is exposed to runtime code
// that wants a build-time signal in addition to the runtime `isTauri()`
// check. Default (`undefined` / `desktop`) keeps the historical behavior.
const buildTarget = (process.env.VITE_OPENHUMAN_TARGET ?? "desktop").trim();
const isWebTarget = buildTarget === "web";

// https://vite.dev/config/
export default defineConfig(async () => ({
  root: "src",
  publicDir: "../public",
  // Read env files from the repo root (not `app/src/`, which is the vite
  // `root` and would be the default `envDir`). Lets `pnpm dev:app` pick up
  // `VITE_BACKEND_URL` / `VITE_OPENHUMAN_APP_ENV` from the same root `.env`
  // the Rust shell uses, instead of needing a separate `app/.env.local`.
  // Without this, `import.meta.env.VITE_*` is empty in dev (Vite does not
  // inherit `process.env` for VITE_-prefixed vars), so `BACKEND_URL` falls
  // through to the production fallback in `src/utils/config.ts` even when
  // the shell exports staging URLs.
  envDir: resolve(__dirname, ".."),
  build: {
    outDir: isWebTarget ? "../dist-web" : "../dist",
    emptyOutDir: true,
    // Compatibility floor for the Wry system WebView (#5571). Since the
    // CEF→Wry migration (#5456) the desktop app renders in the OS-provided
    // engine (WKWebView on macOS, WebView2 on Windows, WebKitGTK on Linux),
    // so the bundle must not emit syntax newer than the oldest supported
    // engine. macOS 12 (Monterey) ships WebKit ~15.x, so Safari 15 is the
    // JS/CSS floor — without it Vite emits untranspiled ES2022+ that fails to
    // parse before React mounts, and the window (revealed unconditionally by
    // the Rust shell) is shown blank. Safari 15 is a strict subset of
    // WebView2/WebKitGTK, so the Windows/Linux bundles lose nothing.
    //
    // The target lowers *syntax* only; it never polyfills runtime methods. The
    // bundle still calls WebKit-15.4 APIs (structuredClone, Object.hasOwn,
    // Array.prototype.at), so the real mount floor is WebKit 15.4 = macOS 12.3
    // (bundle.macOS.minimumSystemVersion). The one WebKit-16.4 feature we use
    // in-source — a RegExp lookbehind — is rewritten to a lookahead at its call
    // site (features/share/shareContent.ts) since no Monterey WebKit ships it.
    target: "safari15",
    cssTarget: "safari15",
    // Desktop CEF has surfaced a runtime where `link.relList.supports` is
    // truthy but not callable. Vite calls it both in the modulepreload
    // polyfill and the dynamic-import preload helper, before React mounts.
    modulePreload: false,
    // Emit source maps so @sentry/vite-plugin can upload them; the plugin
    // deletes the on-disk .map files after upload so users don't receive
    // them in the shipped bundle.
    sourcemap: true,
  },
  plugins: [
    nodePolyfills({
      include: ["buffer", "process", "util", "os", "crypto", "stream"],
      globals: {
        Buffer: true,
        process: true,
        global: true,
      },
    }),
    guardCefRelListSupportsPlugin(),
    devConnectPlugin(),
    react(),
    maybeSentryPlugin(),
  ].filter(Boolean) as PluginOption[],

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: devPort,
    strictPort: true,
    // `false` lets Vite pick its own loopback default; on Windows that lands
    // on `::1` only, leaving 127.0.0.1 unbound. The Tauri dev-server proxy
    // (vendored tauri-cef, reqwest under the hood) resolves `localhost` and
    // can pick either stack — when it picks 127.0.0.1 the request fails,
    // which surfaces as a blank webview / white screen because the SPA
    // bundle never loads. `true` maps to `server.listen('0.0.0.0')` in Vite,
    // binding **every network adapter** (loopback + LAN) so whichever stack
    // reqwest's DNS picks for `localhost` has a listener. Side effect: the
    // dev HMR websocket and bundled sources are reachable from other
    // machines on the same network — fine for `tauri dev`, but on a shared
    // or corporate Wi-Fi consider overriding with `host: 'localhost'` (and
    // accepting the dual-stack hazard) instead. Production builds are
    // unaffected.
    host: host || true,
    allowedHosts: [
      "frontend-runner-openhuman-git-main-vezuresxyz.vercel.app",
    ],
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: hmrPort,
        }
      : {
          // Tauri CEF loads the app from tauri.localhost; without this the
          // HMR client tries ws://tauri.localhost/ and gets ERR_CONNECTION_REFUSED.
          // Force the client to connect to the Vite dev server directly.
          protocol: "ws",
          host: "localhost",
          port: devPort,
          clientPort: devPort,
        },
    watch: {
      // 3. tell Vite to ignore watching `src-tauri` directory (includes src-tauri/ai)
      ignored: ["**/src-tauri/**"],
    },
  },
  resolve: {
    alias: {
      "@": resolve(__dirname, "src"),
      buffer: "buffer",
      process: "process/browser",
      util: "util",
      os: "os-browserify/browser",
    },
  },
  optimizeDeps: {
    include: ["buffer", "process", "util", "os-browserify"],
  },
}));
