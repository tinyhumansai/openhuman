#!/usr/bin/env node
// Stage the registry-pinned Windows modules as Tauri resources. Downloading is
// a build-time operation; a shipped installer needs no GitHub access to load.
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { parseAllList, parseRecords } from "../lib/module-pins.mjs";
import {
  parseReleaseUrls,
  readRegistrySource,
  resolveTestModuleAssets,
} from "../ci/self-hosted/test-module-assets.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const HOST_KEY = "windows-2022-x86_64";
const OUTPUT = join(ROOT, "crates/openhuman-app/bundled-modules");

export function bundledAssets(source) {
  const names = parseAllList(source);
  const records = parseRecords(source);
  const urls = parseReleaseUrls(source);
  const ids = names.map((name) => {
    const record = records.get(name);
    if (!record) throw new Error(`missing module record ${name}`);
    if (!urls.has(record.id)) throw new Error(`missing release URL for ${record.id}`);
    return record.id;
  });
  const assets = resolveTestModuleAssets(source, HOST_KEY, ids);
  return assets.map((asset) => {
    const record = [...records.values()].find((r) => r.id === asset.id);
    for (const component of [record.id, record.version, HOST_KEY, asset.archive]) {
      if (!/^[a-zA-Z0-9][a-zA-Z0-9._-]*$/.test(component) || component === "..") {
        throw new Error(`unsafe registry path component for ${record.id}`);
      }
    }
    return { ...asset, version: record.version, hostKey: HOST_KEY };
  });
}

function dllsUnder(dir) {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return dllsUnder(path);
    return entry.isFile() && entry.name.toLowerCase().endsWith(".dll")
      ? [path]
      : [];
  });
}

export function extractWindowsZip(archive, dir) {
  // ZipFile.ExtractToDirectory rejects entries that escape the destination.
  // It also avoids Git Bash's tar.exe, which may not understand drive paths.
  execFileSync(
    "powershell.exe",
    [
      "-NoProfile",
      "-NonInteractive",
      "-Command",
      "$ErrorActionPreference = 'Stop'; try { Add-Type -AssemblyName System.IO.Compression.FileSystem; [System.IO.Compression.ZipFile]::ExtractToDirectory($env:OPENHUMAN_MODULE_ARCHIVE, $env:OPENHUMAN_MODULE_DESTINATION) } catch { [Console]::Error.WriteLine($_.Exception.Message); exit 1 }",
    ],
    {
      env: {
        ...process.env,
        OPENHUMAN_MODULE_ARCHIVE: archive,
        OPENHUMAN_MODULE_DESTINATION: dir,
      },
      stdio: "pipe",
    },
  );
}

export function stageWindowsModules(output = OUTPUT) {
  if (process.platform !== "win32") {
    throw new Error("stage-windows-modules must run on a Windows build runner");
  }
  const assets = bundledAssets(readRegistrySource());
  rmSync(output, { recursive: true, force: true });
  mkdirSync(output, { recursive: true });
  writeFileSync(join(output, ".gitkeep"), "");
  for (const asset of assets) {
    const dir = join(output, asset.id, asset.version, asset.hostKey);
    mkdirSync(dir, { recursive: true });
    const archive = join(dir, asset.archive);
    execFileSync("curl.exe", ["--fail", "--location", "--retry", "3", "--output", archive, asset.url], {
      stdio: "inherit",
    });
    const actual = createHash("sha256").update(readFileSync(archive)).digest("hex");
    if (actual !== asset.sha256.toLowerCase()) {
      throw new Error(`${asset.id}: downloaded archive does not match the compiled registry pin`);
    }
    extractWindowsZip(archive, dir);
    if (dllsUnder(dir).length !== 1) {
      throw new Error(`${asset.id}: expected exactly one DLL in the release archive`);
    }
    console.log(`[bundled-modules] staged ${asset.id} ${asset.version} (${asset.hostKey})`);
  }
  console.log(`[bundled-modules] staged ${assets.length} verified releases`);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  stageWindowsModules();
}
