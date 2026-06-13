#!/usr/bin/env node
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
let distDir = resolve(root, "app/dist");

for (let i = 2; i < process.argv.length; i += 1) {
  const arg = process.argv[i];
  if (arg === "--dist") {
    const value = process.argv[i + 1];
    if (!value || value.startsWith("-")) {
      console.error("verify-i18n-bundle: --dist requires a path");
      process.exit(2);
    }
    distDir = resolve(process.cwd(), value);
    i += 1;
  } else if (arg === "--help" || arg === "-h") {
    console.log("Usage: node scripts/verify-i18n-bundle.mjs [--dist app/dist]");
    process.exit(0);
  } else {
    console.error(`verify-i18n-bundle: unknown argument: ${arg}`);
    process.exit(2);
  }
}

function listJsFiles(dir) {
  const out = [];
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry);
    const stat = statSync(path);
    if (stat.isDirectory()) {
      out.push(...listJsFiles(path));
    } else if (entry.endsWith(".js")) {
      out.push(path);
    }
  }
  return out;
}

const requiredMarkers = [
  {
    label: "zh-CN locale key",
    needles: ["zh-CN"],
  },
  {
    label: "Simplified Chinese picker label",
    needles: ["\u7b80\u4f53\u4e2d\u6587", "\\u7b80\\u4f53\\u4e2d\\u6587"],
  },
];

if (!existsSync(distDir) || !statSync(distDir).isDirectory()) {
  console.error(
    `verify-i18n-bundle: dist directory does not exist or is not a directory: ${distDir}`,
  );
  process.exit(1);
}

const files = listJsFiles(distDir);
if (files.length === 0) {
  console.error(
    `verify-i18n-bundle: no JavaScript assets found under ${distDir}`,
  );
  process.exit(1);
}

const bundle = files.map((file) => readFileSync(file, "utf8")).join("\n");
const missing = requiredMarkers.filter(
  (marker) => !marker.needles.some((needle) => bundle.includes(needle)),
);

if (missing.length > 0) {
  console.error(
    "verify-i18n-bundle: production bundle is missing i18n markers:",
  );
  for (const marker of missing) {
    console.error(`  - ${marker.label}`);
  }
  process.exit(1);
}

console.log(
  `verify-i18n-bundle: found ${requiredMarkers.length} required markers in ${files.length} JS assets`,
);
