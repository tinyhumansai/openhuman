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
  closeSync,
  copyFileSync,
  existsSync,
  lstatSync,
  mkdirSync,
  openSync,
  readFileSync,
  readSync,
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

// Visual C++ runtime files that must ship beside every Windows module. tinybus
// loads with LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
// so Windows resolves dependencies only beside the module and in System32 —
// never PATH, never the app folder, never a sibling module's dir. VCRUNTIME140
// is not in System32 by default on fresh Windows 10/11, so without this file
// a user without the VC++ 2015-2022 Redistributable installed gets a terminal
// load failure (`dynamic loader rejected the artifact — Win32 error 126`,
// Sentry TAURI-RUST-14J5 / issue #8499). Universal CRT shims
// (`api-ms-win-crt-*.dll`, `ucrtbase.dll`) ship with the OS on Windows 10+
// and need no app-local copy. Keep this list minimal — every entry inflates
// each Windows host_key by its byte count and must be license-compliant for
// app-local redistribution.
const WINDOWS_APP_LOCAL_RUNTIME_FILES = ["vcruntime140.dll"];

// PE/COFF machine type constants. tinybus loader admits a module by the host
// candidate list, so an ARM64 bundle whose VC runtime is x64 (or vice versa)
// resolves beside the module and still fails at `LoadLibraryExW`. We refuse
// a mismatched source before copying so the pin-drift never ships.
const PE_MACHINE = Object.freeze({ x64: 0x8664, arm64: 0xaa64 });

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
    // App-local VC runtime files staged by stageWindowsAppLocalRuntime are not
    // modules; skip them so the "exactly one native library" invariant below
    // still holds no matter when they're placed in `dir`.
    if (WINDOWS_APP_LOCAL_RUNTIME_FILES.includes(name)) return [];
    return entry.isFile() && LIBRARY_EXTENSIONS.some((ext) => name.endsWith(ext))
      ? [path]
      : [];
  });
}

/** True for the Windows release host_keys (e.g. `windows-2022-x86_64`). */
export function isWindowsHostKey(hostKey) {
  return hostKey.startsWith("windows-");
}

/** The redist subfolder name for a Windows host_key's arch. */
function vcRuntimeArch(hostKey) {
  return hostKey.endsWith("-arm64") ? "arm64" : "x64";
}

/**
 * Read a Windows PE's COFF machine field. Returns the raw UInt16 (e.g. 0x8664
 * for x86_64, 0xAA64 for ARM64). The call opens the first 1 KiB of the file
 * only; it never maps or executes the image.
 */
export function peMachine(path) {
  const fd = openSync(path, "r");
  try {
    const buf = Buffer.alloc(0x400);
    const read = readSync(fd, buf, 0, buf.length, 0);
    if (read < 0x40) throw new Error(`${path}: too small to be a PE image`);
    if (buf.slice(0, 2).toString("ascii") !== "MZ") {
      throw new Error(`${path}: missing MZ signature; not a PE image`);
    }
    const peOffset = buf.readUInt32LE(0x3c);
    if (peOffset + 6 > read) throw new Error(`${path}: PE header past read window`);
    if (buf.slice(peOffset, peOffset + 4).toString("latin1") !== "PE\0\0") {
      throw new Error(`${path}: missing PE\\0\\0 signature at e_lfanew`);
    }
    return buf.readUInt16LE(peOffset + 4);
  } finally {
    closeSync(fd);
  }
}

/**
 * Verify `path` is Authenticode-signed by Microsoft on a Windows build host.
 * On non-Windows build hosts this is a best-effort check we cannot run without
 * `osslsigncode`, so we skip with a notice; the PE-machine check still runs
 * and the `OPENHUMAN_VC_REDIST_DIR` contract requires an operator-sourced
 * directory. On Windows this invokes `Get-AuthenticodeSignature` and refuses
 * anything whose signer subject does not include the full Organization
 * attribute `O=Microsoft Corporation` (matched anchored to its X.500
 * comma delimiters — see the regex below for the rationale).
 */
/**
 * Pick a PowerShell host for Authenticode validation. PowerShell 7 (`pwsh.exe`)
 * autoloads `Microsoft.PowerShell.Security` reliably, including when launched
 * from Node's `execFileSync`; Windows PowerShell 5.1 (`powershell.exe`) can
 * fail with `CouldNotAutoloadMatchingModule` on the hosted windows-latest
 * runners (observed during Module bundle CI). Prefer `pwsh.exe` where
 * available; fall back to `powershell.exe` with an explicit `Import-Module`.
 */
function resolvePowerShellHost() {
  for (const candidate of ["pwsh.exe", "powershell.exe"]) {
    try {
      execFileSync(candidate, ["-NoProfile", "-Command", "exit 0"], { stdio: "ignore" });
      return candidate;
    } catch {
      // try next
    }
  }
  throw new Error(
    "neither pwsh.exe nor powershell.exe is available for Authenticode validation",
  );
}

export function assertMicrosoftSigned(path) {
  // Test-only escape: setting OPENHUMAN_SKIP_AUTHENTICODE_CHECK=1 bypasses the
  // signer check so unit tests can use a hand-rolled PE stub. This is NEVER
  // set by any release workflow; a repository-wide guard
  // (`scripts/ci/check-no-authenticode-skip.mjs` could enforce it) would fail
  // CI if it ever appeared outside tests. Document as such.
  if (process.env.OPENHUMAN_SKIP_AUTHENTICODE_CHECK === "1") {
    console.warn(
      `[bundled-modules] OPENHUMAN_SKIP_AUTHENTICODE_CHECK=1 — skipping Authenticode check for ${path}. ` +
        `This must never be set by a release workflow.`,
    );
    return;
  }
  if (process.platform !== "win32") {
    console.warn(
      `[bundled-modules] skipping Authenticode check for ${path} on ${process.platform}; ` +
        `verify the source matches a Microsoft redistributable on the build host.`,
    );
    return;
  }
  const host = resolvePowerShellHost();
  // Import-Module ensures the Security cmdlets are available on Windows
  // PowerShell 5.1, which can fail to autoload the module under execFileSync.
  // On pwsh.exe it's a cheap no-op since autoload would have found it anyway.
  const subject = execFileSync(
    host,
    [
      "-NoProfile",
      "-NonInteractive",
      "-Command",
      "$ErrorActionPreference = 'Stop'; " +
        "Import-Module Microsoft.PowerShell.Security -ErrorAction SilentlyContinue; " +
        "$s = Get-AuthenticodeSignature -LiteralPath $env:OPENHUMAN_AUTHENTICODE_PATH; " +
        "if ($s.Status -ne 'Valid') { throw \"signature status $($s.Status): $($s.StatusMessage)\" }; " +
        "if (-not $s.SignerCertificate) { throw 'no signer certificate' }; " +
        "$s.SignerCertificate.Subject",
    ],
    {
      env: { ...process.env, OPENHUMAN_AUTHENTICODE_PATH: path },
      stdio: ["ignore", "pipe", "pipe"],
      encoding: "utf8",
    },
  ).trim();
  // Match on the Organization attribute, anchored to the whole X.500 field.
  // The CN varies across Microsoft code-signing certificates — the VC redist's
  // vcruntime140.dll ships with
  //   `CN=Microsoft Windows Software Compatibility Publisher, O=Microsoft
  //    Corporation, L=Redmond, S=Washington, C=US`
  // (observed in CI), and other MS-signed artifacts use CNs like
  // `Microsoft Corporation` or `Microsoft Windows`. The Organization attribute
  // (`O=Microsoft Corporation`) is the stable discriminator across all MS
  // code-signing identities. Anchoring on comma-or-start and comma-or-end
  // prevents a `O=MicrosoftEvil` or `O=Microsoft Corporation Clone Ltd` from
  // passing. An unanchored CN substring on an attribute whose value we don't
  // control was CodeRabbit's correct concern; applying it to CN was the wrong
  // attribute.
  if (!/(?:^|,\s*)O=Microsoft Corporation(?:,|$)/.test(subject)) {
    throw new Error(
      `${path}: Authenticode subject does not include "O=Microsoft Corporation" (${subject}); refusing to ship into bundled-modules.`,
    );
  }
}

/**
 * Reject a staged VC runtime file whose arch does not match the module's.
 * Shipping an x64 vcruntime140.dll beside an ARM64 module leaves the original
 * `LoadLibraryExW` failure in place because the dynamic loader rejects a
 * mismatched machine type — the fix must be arch-correct or it is no fix.
 */
export function assertRuntimeMatchesHost(path, hostKey) {
  const arch = vcRuntimeArch(hostKey);
  const expected = PE_MACHINE[arch];
  const actual = peMachine(path);
  if (actual !== expected) {
    throw new Error(
      `${path}: PE machine 0x${actual.toString(16)} does not match ${hostKey} (expected 0x${expected.toString(16)}). ` +
        `Set OPENHUMAN_VC_REDIST_DIR to a directory with the ${arch} VC runtime.`,
    );
  }
}

/**
 * Resolve the directory on this build host that contains the Visual C++
 * redistributable DLLs for `hostKey`'s arch. Tried in order:
 *
 *  1. `OPENHUMAN_VC_REDIST_DIR` — explicit override; the environment variable
 *     points at a directory containing the files listed in
 *     `WINDOWS_APP_LOCAL_RUNTIME_FILES`. Used by CI jobs that assemble the
 *     redist dir themselves (e.g. extracted from a pinned VC_redist.x64.exe).
 *  2. `VCToolsRedistDir` — set by a Visual Studio Developer Prompt. Points at
 *     `.../VC/Redist/MSVC/<version>/`; the per-arch `Microsoft.VC143.CRT` dir
 *     under it holds the files.
 *  3. `vswhere.exe` — probes the installed Visual Studio and walks
 *     `VC/Redist/MSVC/<newest>/<arch>/Microsoft.VC143.CRT`. Windows build host
 *     only; the probe is skipped elsewhere so a cross-build from Linux fails
 *     fast with an actionable message rather than silently shipping no runtime.
 *
 * Throws with a message that names the fix on every miss.
 */
export function findWindowsVcRuntimeDir(hostKey) {
  if (!isWindowsHostKey(hostKey)) {
    throw new Error(`findWindowsVcRuntimeDir called with non-Windows host_key ${hostKey}`);
  }
  const arch = vcRuntimeArch(hostKey);
  // Grep-friendly prefix for every resolution outcome so a CI failure that
  // traces back to the wrong CRT (older VC14x, Spectre-adjacent, ARM64 mix-up)
  // is one `grep` away in the job log.
  const log = (branch, dir) =>
    console.log(`[bundled-modules] VC runtime for ${hostKey}: ${branch} -> ${dir}`);

  const override = process.env.OPENHUMAN_VC_REDIST_DIR;
  if (override) {
    if (!existsSync(join(override, WINDOWS_APP_LOCAL_RUNTIME_FILES[0]))) {
      throw new Error(
        `OPENHUMAN_VC_REDIST_DIR=${override} does not contain ${WINDOWS_APP_LOCAL_RUNTIME_FILES[0]}`,
      );
    }
    log("OPENHUMAN_VC_REDIST_DIR", override);
    return override;
  }

  const toolsRedist = process.env.VCToolsRedistDir;
  if (toolsRedist) {
    const crtDir = join(toolsRedist, arch, "Microsoft.VC143.CRT");
    if (existsSync(join(crtDir, WINDOWS_APP_LOCAL_RUNTIME_FILES[0]))) {
      log("VCToolsRedistDir", crtDir);
      return crtDir;
    }
  }

  if (process.platform !== "win32") {
    const redistExe = arch === "arm64" ? "VC_redist.arm64.exe" : "VC_redist.x64.exe";
    throw new Error(
      `cannot locate the Windows VC++ runtime for ${hostKey} on a ${process.platform} build host. ` +
        `Set OPENHUMAN_VC_REDIST_DIR to a directory containing the ${arch} ${WINDOWS_APP_LOCAL_RUNTIME_FILES.join(", ")} ` +
        `(e.g. extracted from ${redistExe}), or stage on a Windows runner where VCToolsRedistDir is set.`,
    );
  }

  const programFilesX86 = process.env["ProgramFiles(x86)"] ?? "C:\\Program Files (x86)";
  const vswhere = join(programFilesX86, "Microsoft Visual Studio", "Installer", "vswhere.exe");
  if (!existsSync(vswhere)) {
    throw new Error(
      `${vswhere} not found; install Visual Studio Build Tools on the Windows runner, or set ` +
        `OPENHUMAN_VC_REDIST_DIR to a directory containing the ${arch} ${WINDOWS_APP_LOCAL_RUNTIME_FILES.join(", ")}.`,
    );
  }
  // -products * admits standalone Build Tools alongside the full IDE; -requires
  // picks the install that actually has the C++ compiler for this target arch,
  // not just the newest VS. Mirrors scripts/run-dev-win.sh:100.
  const vswhereRequires =
    arch === "arm64"
      ? "Microsoft.VisualStudio.Component.VC.Tools.ARM64"
      : "Microsoft.VisualStudio.Component.VC.Tools.x86.x64";
  const installPath = execFileSync(
    vswhere,
    [
      "-latest",
      "-products",
      "*",
      "-requires",
      vswhereRequires,
      "-property",
      "installationPath",
    ],
    { stdio: ["ignore", "pipe", "pipe"], encoding: "utf8" },
  ).trim();
  if (!installPath) {
    throw new Error(
      `vswhere.exe reported no Visual Studio installation with ${vswhereRequires}. ` +
        `Install that workload, or set OPENHUMAN_VC_REDIST_DIR to a directory with the ${arch} VC runtime.`,
    );
  }

  const redistRoot = join(installPath, "VC", "Redist", "MSVC");
  if (!existsSync(redistRoot)) {
    throw new Error(
      `${redistRoot} missing; install the "MSVC v143 - VS 2022 C++ x64/x86 build tools" workload or set OPENHUMAN_VC_REDIST_DIR.`,
    );
  }
  // Walk every <version>/<arch>/<any>/ dir under VC/Redist/MSVC and admit the
  // first one that holds our runtime files. The fast path matches VC143 by
  // name; the fallback accepts any Microsoft.VC14*.CRT (VS 2026 ships VC144+
  // under the same layout) and tolerates vendor-specific suffixes like
  // "Microsoft.VC143.CRT.Spectre" that the Spectre-mitigated redist workloads
  // install alongside the plain one. Newest redist version wins.
  const versions = readdirSync(redistRoot, { withFileTypes: true })
    .filter((e) => e.isDirectory())
    .map((e) => e.name)
    .sort((a, b) => (a < b ? 1 : a > b ? -1 : 0));
  for (const version of versions) {
    const archDir = join(redistRoot, version, arch);
    if (!existsSync(archDir)) continue;
    const fastPath = join(archDir, "Microsoft.VC143.CRT");
    if (existsSync(join(fastPath, WINDOWS_APP_LOCAL_RUNTIME_FILES[0]))) {
      log("vswhere+VC143.CRT", fastPath);
      return fastPath;
    }
    for (const entry of readdirSync(archDir, { withFileTypes: true })) {
      if (!entry.isDirectory()) continue;
      // Prefer non-Spectre CRT subdirs so we never ship Spectre-mitigated bits
      // to users who did not opt into them (the plain CRT is MS-signed and
      // ABI-identical for app-local deployment).
      if (!/^Microsoft\.VC14[0-9]+\.CRT$/.test(entry.name)) continue;
      const candidate = join(archDir, entry.name);
      if (existsSync(join(candidate, WINDOWS_APP_LOCAL_RUNTIME_FILES[0]))) {
        log(`vswhere+${entry.name}`, candidate);
        return candidate;
      }
    }
  }
  throw new Error(
    `no Microsoft.VC14*.CRT/${WINDOWS_APP_LOCAL_RUNTIME_FILES[0]} for ${arch} under ${redistRoot}. ` +
      `Install the "MSVC v14x - ${arch === "arm64" ? "ARM64/ARM64EC" : "x64/x86"} build tools" ` +
      `workload via vs_installer, or set OPENHUMAN_VC_REDIST_DIR to a directory containing the file.`,
  );
}

/**
 * Place `WINDOWS_APP_LOCAL_RUNTIME_FILES` beside the staged module library on
 * Windows. No-op on other host_keys. Returns the staged absolute paths.
 *
 * `libraryDir` is the directory holding the extracted native module library,
 * not the archive's extraction root — a release may stage its library in a
 * subdirectory (`librariesUnder` finds it), and the tinybus loader uses
 * LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR which only resolves dependencies in that
 * same subdirectory.
 *
 * Each candidate runtime file is validated BEFORE copy: its PE machine must
 * match `hostKey`'s arch (so an x64 vcruntime140.dll never lands next to an
 * ARM64 module), and on Windows build hosts its Authenticode signature must
 * resolve to a Microsoft signer (defense in depth over `OPENHUMAN_VC_REDIST_DIR`,
 * which an operator supplies). Both checks run against the resolved source,
 * then the destination copy is re-checked to catch write-time tampering.
 *
 * This closes issue #8499 / Sentry TAURI-RUST-14J5: before this step the
 * installer shipped tinyconnectors.dll (and every other `windows-*` tiny
 * module) alone; the tinybus loader uses LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR |
 * LOAD_LIBRARY_SEARCH_SYSTEM32, which only resolves dependencies beside the
 * module or in System32. On a user without VC++ 2015-2022 Redistributable
 * installed, VCRUNTIME140.dll is in neither place and `LoadLibraryExW`
 * returns NULL with GetLastError() == 126.
 */
export function stageWindowsAppLocalRuntime(libraryDir, hostKey) {
  if (!isWindowsHostKey(hostKey)) return [];
  const source = findWindowsVcRuntimeDir(hostKey);
  const copied = [];
  for (const name of WINDOWS_APP_LOCAL_RUNTIME_FILES) {
    const src = join(source, name);
    if (!existsSync(src)) {
      throw new Error(`VC runtime file missing: ${src}`);
    }
    assertRuntimeMatchesHost(src, hostKey);
    assertMicrosoftSigned(src);
    const dst = join(libraryDir, name);
    copyFileSync(src, dst);
    assertRuntimeMatchesHost(dst, hostKey);
    copied.push(dst);
  }
  return copied;
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
    const libraries = librariesUnder(dir);
    if (libraries.length !== 1) {
      throw new Error(`${asset.id}: expected exactly one native library in the release archive`);
    }
    // Windows-only: ship vcruntime140.dll in the SAME directory as the
    // extracted module library — not necessarily `dir` itself, since an
    // archive may stage the library in a subdirectory and the tinybus loader
    // searches only beside the module (LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR).
    const runtime = stageWindowsAppLocalRuntime(dirname(libraries[0]), asset.hostKey);
    if (!keepsArchive(asset.hostKey)) replaceArchiveWithMarker(archive, actual);
    const runtimeNote = runtime.length ? ` + ${runtime.length} runtime file(s)` : "";
    console.log(
      `[bundled-modules] staged ${asset.id} ${asset.version} (${asset.hostKey})${runtimeNote}`,
    );
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
