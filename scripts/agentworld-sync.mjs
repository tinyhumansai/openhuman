#!/usr/bin/env node
/**
 * agentworld-sync.mjs — vendoring/sync script for Agent World UI components.
 *
 * Selectively copies the minimal subset of `tiny.place/website/src` into
 * `app/src/agentworld/vendor/` so the ported hooks and section components
 * work without bundling the full website tree.
 *
 * Wave 0 scope (Explore only):
 *   - pages/explore/         → vendor/pages/explore/
 *   - common/api-context/    → vendor/common/api-context/
 *   - hooks/use-explorer.ts  → vendor/hooks/use-explorer.ts
 *   - hooks/use-search.ts    → vendor/hooks/use-search.ts
 *   - hooks/use-directory.ts → vendor/hooks/use-directory.ts
 *   - store/app/             → vendor/store/app/
 *   - ui/<components>        → vendor/ui/
 *
 * Wave 1+ section agents: append their section's file set to COPY_PATHS.
 *
 * Usage:
 *   node scripts/agentworld-sync.mjs [--dry-run]
 *
 * Requires: Node 18+ (fs/promises, path).
 * The tiny.place repo must be checked out at vendor/tiny.place (submodule).
 */

import { cp, mkdir, readdir, stat } from "node:fs/promises";
import { join, resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = resolve(__dirname, "..");

const SRC_ROOT = join(REPO_ROOT, "vendor/tiny.place/website/src");
const DEST_ROOT = join(REPO_ROOT, "app/src/agentworld/vendor");

const DRY_RUN = process.argv.includes("--dry-run");

// === Wave 0: Explore section only ===
// Wave 1+ section agents append their paths here.
const COPY_PATHS = [
  // Explore section pages
  "pages/explore",
  // Shared API context (must accept injected client — see doc 02 §6)
  "common/api-context",
  // Section-specific hooks
  "hooks/use-explorer.ts",
  "hooks/use-search.ts",
  "hooks/use-directory.ts",
  // App store for section state
  "store/app",
  // UI primitives needed by Explore
  "ui/card",
  "ui/badge",
  "ui/button",
  "ui/skeleton",
];

async function exists(p) {
  try {
    await stat(p);
    return true;
  } catch {
    return false;
  }
}

async function sync() {
  if (!(await exists(SRC_ROOT))) {
    console.error(
      `[agentworld-sync] ERROR: tiny.place website source not found at ${SRC_ROOT}\n` +
        "Run: git submodule update --init vendor/tiny.place",
    );
    process.exit(1);
  }

  console.log(
    `[agentworld-sync] ${DRY_RUN ? "(dry-run) " : ""}syncing to ${DEST_ROOT}`,
  );

  for (const relPath of COPY_PATHS) {
    const src = join(SRC_ROOT, relPath);
    const dest = join(DEST_ROOT, relPath);

    if (!(await exists(src))) {
      console.warn(`[agentworld-sync]   SKIP (not found): ${relPath}`);
      continue;
    }

    if (DRY_RUN) {
      console.log(`[agentworld-sync]   would copy: ${relPath}`);
      continue;
    }

    await mkdir(dirname(dest), { recursive: true });
    await cp(src, dest, { recursive: true, force: true });
    console.log(`[agentworld-sync]   copied: ${relPath}`);
  }

  console.log("[agentworld-sync] done.");
}

sync().catch((err) => {
  console.error("[agentworld-sync] FATAL:", err);
  process.exit(1);
});
