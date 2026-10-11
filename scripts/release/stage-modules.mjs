#!/usr/bin/env node
// Stage the registry-pinned native modules for the host (or `--host-key`) as
// bundled release archives. Downloading is a build-time operation: a shipped
// installer, release tarball or bench image needs no GitHub access to load a
// module. The output layout is `<id>/<version>/<host_key>/<archive>` plus the
// extracted library, which is what `modules::ops::set_bundled_releases_dir`
// (or `OPENHUMAN_BUNDLED_MODULES`) points at. On macOS the archive is replaced
// by `<archive>.sha256` holding its verified pin (see `keepsArchive`).
//
// Usage: node scripts/release/stage-modules.mjs [--target TRIPLE | --host-key KEY] [--output DIR]
//
// `--target` takes the Rust target triple being built, so a cross-built app
// (x86_64 macOS on an arm64 runner) gets modules for the target, not the runner.
//
// One key per OS/arch is staged: the oldest published build, because tinybus
// tries every candidate for the host in order and skips any that is not
// bundled, so the lowest glibc / macOS floor runs on every newer host too.
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import {
  chmodSync,
  lstatSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { parseAllList, parseRecords } from "../lib/module-pins.mjs";
import {
  parseReleaseUrls,
  readRegistrySource,
  resolveTestModuleAssets,
} from "../ci/self-hosted/test-module-assets.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const OUTPUT = join(ROOT, "crates/openhuman-app/bundled-modules");
const LIBRARY_EXTENSIONS = [".dll", ".so", ".dylib"];

/** The registry host key to bundle for a Node `platform`/`arch` pair. */
export function defaultHostKey(platform = process.platform, arch = process.arch) {
  const archKey = { x64: "x86_64", arm64: "arm64" }[arch];
  if (!archKey) throw new Error(`no bundled modules for architecture ${arch}`);
  switch (platform) {
    case "darwin":
      return `macos-15-${archKey}`;
    case "linux":
      return `ubuntu-22.04-${archKey}`;
    case "win32":
      return archKey === "arm64" ? "windows-11-arm64" : "windows-2022-x86_64";
    default:
      throw new Error(`no bundled modules for platform ${platform}`);
  }
}

/** The registry host key to bundle for a Rust target triple. */
export function hostKeyForTarget(triple) {
  const arch = triple.split("-")[0];
  const archKey = { x86_64: "x86_64", aarch64: "arm64" }[arch];
  if (!archKey) throw new Error(`no bundled modules for target ${triple}`);
  if (triple.includes("apple-darwin")) return `macos-15-${archKey}`;
  if (triple.includes("linux")) return `ubuntu-22.04-${archKey}`;
  if (triple.includes("windows")) {
    return archKey === "arm64" ? "windows-11-arm64" : "windows-2022-x86_64";
  }
  throw new Error(`no bundled modules for target ${triple}`);
}

/**
 * Whether the staged entry keeps its release archive. A notarized macOS app
 * cannot: Apple's notary service unpacks it and rejects the unsigned Mach-O
 * inside, and signing them would change the pinned bytes. There the archive is
 * replaced by its verified digest, which tinybus admits from the installer
 * bundle only; the extracted files are signed and sealed with the app.
 */
export function keepsArchive(hostKey) {
  return !hostKey.startsWith("macos-");
}

/** Replace a verified `archive` with `<archive>.sha256` holding `sha256`. */
export function replaceArchiveWithMarker(archive, sha256) {
  writeFileSync(`${archive}.sha256`, `${sha256}\n`);
  rmSync(archive);
}

export function bundledAssets(source, hostKey) {
  const names = parseAllList(source);
  const records = parseRecords(source);
  const urls = parseReleaseUrls(source);
  const ids = names.map((name) => {
    const record = records.get(name);
    if (!record) throw new Error(`missing module record ${name}`);
    if (!urls.has(record.id)) throw new Error(`missing release URL for ${record.id}`);
    return record.id;
  });
  // A module that publishes nothing for this OS is not bundled there;
  // one that publishes the OS but not this key is
  // a registry hole and fails below.
  const osPrefix = hostKey.split("-")[0];
  const byId = new Map([...records.values()].map((r) => [r.id, r]));
  const wanted = ids.filter((id) =>
    byId.get(id).assets.some((a) => a.hostKey.startsWith(`${osPrefix}-`)),
  );
  const assets = resolveTestModuleAssets(source, hostKey, wanted);
  return assets.map((asset) => {
    const record = [...records.values()].find((r) => r.id === asset.id);
    for (const component of [record.id, record.version, hostKey, asset.archive]) {
      if (!/^[a-zA-Z0-9][a-zA-Z0-9._-]*$/.test(component) || component === "..") {
        throw new Error(`unsafe registry path component for ${record.id}`);
      }
    }
    return { ...asset, version: record.version, hostKey };
  });
}

function librariesUnder(dir) {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return librariesUnder(path);
    const name = entry.name.toLowerCase();
    return entry.isFile() && LIBRARY_EXTENSIONS.some((ext) => name.endsWith(ext))
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

/** Extract `archive` into `dir`; GNU and BSD tar both refuse `..` entries. */
export function extractArchive(archive, dir) {
  if (archive.endsWith(".zip")) {
    if (process.platform === "win32") return extractWindowsZip(archive, dir);
    execFileSync("unzip", ["-q", "-o", archive, "-d", dir], { stdio: "pipe" });
    return;
  }
  execFileSync("tar", ["-xzf", archive, "-C", dir], { stdio: "pipe" });
}

/**
 * Reset every directory under `root` (and `root` itself) to 0755 and every
 * regular file to 0644. tinybus refuses a module whose directory, or any
 * ancestor of it, another account can write ("module directory is writable by
 * another user"), and the staged tree ships in the AppImage/deb as-is, so it
 * must not inherit the build host's umask (002 on Ubuntu) or the release
 * tarball's mode bits. Libraries need no execute bit: `dlopen` maps them
 * without it, and Debian policy ships shared objects 0644. Symlinks are left
 * alone; `chmod` would follow them out of the tree.
 */
export function normalizeStagedPermissions(root) {
  const stat = lstatSync(root);
  if (stat.isDirectory()) {
    chmodSync(root, 0o755);
    for (const name of readdirSync(root)) normalizeStagedPermissions(join(root, name));
  } else if (stat.isFile()) {
    chmodSync(root, 0o644);
  }
}

const DOWNLOAD_TIMEOUT_MS = 120_000;

/**
 * Fetch `url` to `destination` byte-for-byte. Identity encoding is requested
 * and anything else is refused: `fetch` would transparently decode a
 * `Content-Encoding: gzip` body, and the registry pin covers the archive's
 * own bytes. Each attempt has a deadline that also covers the body.
 */
export async function download(url, destination, timeoutMs = DOWNLOAD_TIMEOUT_MS) {
  let lastError;
  for (let attempt = 1; attempt <= 3; attempt += 1) {
    try {
      const response = await fetch(url, {
        redirect: "follow",
        headers: { "accept-encoding": "identity" },
        signal: AbortSignal.timeout(timeoutMs),
      });
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      const encoding = response.headers.get("content-encoding");
      if (encoding && encoding.toLowerCase() !== "identity") {
        throw new Error(`unexpected Content-Encoding ${encoding}`);
      }
      writeFileSync(destination, Buffer.from(await response.arrayBuffer()));
      return;
    } catch (error) {
      lastError = error;
    }
  }
  throw new Error(`download of ${url} failed: ${lastError}`);
}

export async function stageModules({
  hostKey = defaultHostKey(),
  output = OUTPUT,
  assets = bundledAssets(readRegistrySource(), hostKey),
} = {}) {
  rmSync(output, { recursive: true, force: true });
  mkdirSync(output, { recursive: true });
  writeFileSync(join(output, ".gitkeep"), "");
  for (const asset of assets) {
    const dir = join(output, asset.id, asset.version, asset.hostKey);
    mkdirSync(dir, { recursive: true });
    const archive = join(dir, asset.archive);
    await download(asset.url, archive);
    const actual = createHash("sha256").update(readFileSync(archive)).digest("hex");
    if (actual !== asset.sha256.toLowerCase()) {
      throw new Error(`${asset.id}: downloaded archive does not match the compiled registry pin`);
    }
    extractArchive(archive, dir);
    if (librariesUnder(dir).length !== 1) {
      throw new Error(`${asset.id}: expected exactly one native library in the release archive`);
    }
    if (!keepsArchive(asset.hostKey)) replaceArchiveWithMarker(archive, actual);
    console.log(`[bundled-modules] staged ${asset.id} ${asset.version} (${asset.hostKey})`);
  }
  normalizeStagedPermissions(output);
  console.log(`[bundled-modules] staged ${assets.length} verified releases for ${hostKey}`);
  return output;
}

function option(name) {
  const index = process.argv.indexOf(name);
  return index === -1 ? undefined : process.argv[index + 1];
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const target = option("--target");
  await stageModules({
    hostKey: option("--host-key") ?? (target ? hostKeyForTarget(target) : defaultHostKey()),
    output: option("--output") ? resolve(option("--output")) : OUTPUT,
  });
}
