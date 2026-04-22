import { sentryVitePlugin } from "@sentry/vite-plugin";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";
import { nodePolyfills } from "vite-plugin-node-polyfills";

const host = process.env.TAURI_DEV_HOST;

// Only upload source maps when CI provides the Sentry auth token. Local
// `yarn build` and PRs without secret access are unaffected — the plugin
// is simply not registered.
const sentryAuthToken = process.env.SENTRY_AUTH_TOKEN;
const sentryOrg = process.env.SENTRY_ORG;
const sentryProject = process.env.SENTRY_PROJECT;
const sentryRelease = process.env.SENTRY_RELEASE;
const shouldUploadSourceMaps = Boolean(
  sentryAuthToken && sentryOrg && sentryProject
);

// https://vite.dev/config/
export default defineConfig(async () => ({
  root: "src",
  publicDir: "../public",
  build: {
    outDir: "../dist",
    emptyOutDir: true,
    // Emit source maps only when uploading to Sentry. Keeps local / PR
    // builds fast and avoids the ~20 MB map files pushing Node past its
    // default heap.
    sourcemap: shouldUploadSourceMaps,
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
    shouldUploadSourceMaps
      ? sentryVitePlugin({
          authToken: sentryAuthToken,
          org: sentryOrg,
          project: sentryProject,
          release: sentryRelease ? { name: sentryRelease } : undefined,
          sourcemaps: {
            filesToDeleteAfterUpload: ["../dist/**/*.map"],
          },
          telemetry: false,
        })
      : null,
  ],

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
