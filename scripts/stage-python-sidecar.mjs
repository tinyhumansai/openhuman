#!/usr/bin/env node
// Stages a standalone CPython distribution (python-build-standalone) into
// app/src-tauri/python/ so Tauri bundles it as an app resource. End-user
// machines do not need Python installed — this ships a fully relocatable
// interpreter with pip included.
//
// To upgrade Python: bump PYTHON_VERSION and PBS_RELEASE below. Release
// matrix: https://github.com/astral-sh/python-build-standalone/releases
import { spawnSync } from "node:child_process";
import {
  createWriteStream,
  existsSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { Readable } from "node:stream";
import { pipeline } from "node:stream/promises";

const __dirname = dirname(fileURLToPath(import.meta.url));
const root = resolve(__dirname, "..");

const PYTHON_VERSION = "3.12.8";
const PBS_RELEASE = "20241206";

const TRIPLE_MAP = {
  "darwin-x64": "x86_64-apple-darwin",
  "darwin-arm64": "aarch64-apple-darwin",
  "linux-x64": "x86_64-unknown-linux-gnu",
  "linux-arm64": "aarch64-unknown-linux-gnu",
  "win32-x64": "x86_64-pc-windows-msvc",
};

const hostKey = `${process.platform}-${process.arch}`;
const triple = TRIPLE_MAP[hostKey];
if (!triple) {
  console.error(`[python:stage] unsupported host platform: ${hostKey}`);
  process.exit(1);
}

const archiveName = `cpython-${PYTHON_VERSION}+${PBS_RELEASE}-${triple}-install_only.tar.gz`;
const downloadUrl = `https://github.com/astral-sh/python-build-standalone/releases/download/${PBS_RELEASE}/${archiveName}`;

const cacheDir = join(root, ".cache", "python-build-standalone");
const cachedArchive = join(cacheDir, archiveName);
const srcTauriDir = join(root, "app", "src-tauri");
const pythonDir = join(srcTauriDir, "python");
const stampFile = join(pythonDir, ".stamp");
const stampContent = `${PYTHON_VERSION}+${PBS_RELEASE}-${triple}`;

if (existsSync(stampFile)) {
  try {
    const current = readFileSync(stampFile, "utf8").trim();
    if (current === stampContent) {
      console.log(`[python:stage] already staged: ${stampContent}`);
      process.exit(0);
    }
    console.log(
      `[python:stage] stamp mismatch (have "${current}", want "${stampContent}") — restaging`,
    );
  } catch {
    // fall through and restage
  }
}

if (existsSync(pythonDir)) {
  console.log(`[python:stage] removing stale ${pythonDir}`);
  rmSync(pythonDir, { recursive: true, force: true });
}

mkdirSync(cacheDir, { recursive: true });
mkdirSync(srcTauriDir, { recursive: true });

if (!existsSync(cachedArchive)) {
  console.log(`[python:stage] downloading ${downloadUrl}`);
  await downloadFile(downloadUrl, cachedArchive);
} else {
  console.log(`[python:stage] using cached archive ${cachedArchive}`);
}

console.log(`[python:stage] extracting into ${srcTauriDir}`);
// The install_only archives extract to a top-level "python/" directory,
// producing app/src-tauri/python/ with bin/python3 (or python.exe on Windows).
const tarRes = spawnSync("tar", ["-xzf", cachedArchive, "-C", srcTauriDir], {
  stdio: "inherit",
  shell: false,
});
if (tarRes.status !== 0) {
  console.error(`[python:stage] tar extraction failed (status ${tarRes.status})`);
  process.exit(tarRes.status ?? 1);
}

const isWindows = process.platform === "win32";
const pythonExe = isWindows
  ? join(pythonDir, "python.exe")
  : join(pythonDir, "bin", "python3");

if (!existsSync(pythonExe)) {
  console.error(`[python:stage] expected interpreter not found: ${pythonExe}`);
  process.exit(1);
}

writeFileSync(stampFile, stampContent);
console.log(`[python:stage] staged ${stampContent}`);
console.log(`[python:stage] interpreter: ${pythonExe}`);

// Sanity check: report version + confirm pip is present.
const versionRes = spawnSync(pythonExe, ["--version"], { encoding: "utf8" });
if (versionRes.status === 0) {
  console.log(`[python:stage] ${versionRes.stdout.trim() || versionRes.stderr.trim()}`);
}
const pipRes = spawnSync(pythonExe, ["-m", "pip", "--version"], { encoding: "utf8" });
if (pipRes.status === 0) {
  console.log(`[python:stage] ${pipRes.stdout.trim()}`);
} else {
  console.warn(`[python:stage] pip self-check failed — distribution may be incomplete`);
}

async function downloadFile(url, dest) {
  const tmp = `${dest}.part`;
  let currentUrl = url;
  for (let redirects = 0; redirects < 5; redirects += 1) {
    const res = await fetch(currentUrl, { redirect: "manual" });
    if (res.status >= 300 && res.status < 400 && res.headers.get("location")) {
      currentUrl = new URL(res.headers.get("location"), currentUrl).toString();
      continue;
    }
    if (!res.ok) {
      throw new Error(`download failed: ${res.status} ${res.statusText} — ${currentUrl}`);
    }
    await pipeline(Readable.fromWeb(res.body), createWriteStream(tmp));
    // Atomic-ish rename so a partial download never looks complete.
    const { renameSync } = await import("node:fs");
    renameSync(tmp, dest);
    return;
  }
  throw new Error(`too many redirects fetching ${url}`);
}
