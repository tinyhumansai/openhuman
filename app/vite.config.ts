import { defineConfig, type PluginOption } from "vite";
import react from "@vitejs/plugin-react";
import { sentryVitePlugin } from "@sentry/vite-plugin";

import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { nodePolyfills } from "vite-plugin-node-polyfills";

const host = process.env.TAURI_DEV_HOST;

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
    org: process.env.SENTRY_ORG,
    project: process.env.SENTRY_PROJECT,
    release: { name: computeSentryRelease() },
    sourcemaps: {
      // Vite emits hashed asset files under dist/assets/ — upload every
      // .js / .map the build produces.
      assets: ["../dist/**/*.js", "../dist/**/*.map"],
      // Never ship raw .map files to end users; the upload keeps a copy
      // server-side for symbolication while the bundled app strips them.
      filesToDeleteAfterUpload: ["../dist/**/*.map"],
    },
    // Release tagging + commits are handled by sentry-cli / the plugin
    // itself when AUTH_TOKEN and CI env (GITHUB_SHA etc.) are present.
    telemetry: false,
  });
}

// https://vite.dev/config/
export default defineConfig(async () => ({
  root: "src",
  publicDir: "../public",
  build: {
    outDir: "../dist",
    emptyOutDir: true,
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
    react(),
    maybeSentryPlugin(),
  ].filter(Boolean) as PluginOption[],

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    allowedHosts: [
      "frontend-runner-openhuman-git-main-vezuresxyz.vercel.app",
    ],
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri` directory (includes src-tauri/ai)
      ignored: ["**/src-tauri/**"],
    },
  },
  resolve: {
    alias: {
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
