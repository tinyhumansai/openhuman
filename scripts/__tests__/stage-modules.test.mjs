import assert from "node:assert/strict";
import {
  chmodSync,
  existsSync,
  lstatSync,
  mkdtempSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { createHash } from "node:crypto";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createServer } from "node:http";
import { execFileSync } from "node:child_process";
import { crc32, gzipSync } from "node:zlib";
import { test } from "node:test";

import { readRegistrySource } from "../ci/self-hosted/test-module-assets.mjs";
import { parseAllList } from "../lib/module-pins.mjs";
import {
  bundledAssets,
  defaultHostKey,
  download,
  findWindowsVcRuntimeDir,
  hostKeyForTarget,
  extractWindowsZip,
  isWindowsHostKey,
  keepsArchive,
  normalizeStagedPermissions,
  peMachine,
  replaceArchiveWithMarker,
  stageModules,
  stageWindowsAppLocalRuntime,
} from "../release/stage-modules.mjs";

/**
 * Minimal PE/COFF bytes carrying a specific Machine field. Enough for
 * `peMachine` and `assertRuntimeMatchesHost` to parse; not a loadable image.
 */
function minimalPeBytes(machine) {
  const buf = Buffer.alloc(0x100);
  buf.write("MZ", 0, "ascii");
  buf.writeUInt32LE(0x40, 0x3c); // e_lfanew -> PE sig at 0x40
  buf.write("PE\0\0", 0x40, "latin1");
  buf.writeUInt16LE(machine, 0x44); // COFF Machine
  return buf;
}

const PE_X64 = minimalPeBytes(0x8664);
const PE_ARM64 = minimalPeBytes(0xaa64);

const HOST_KEYS = [
  "macos-15-arm64",
  "macos-15-x86_64",
  "ubuntu-22.04-x86_64",
  "ubuntu-22.04-arm64",
  "windows-2022-x86_64",
  "windows-11-arm64",
];

for (const hostKey of HOST_KEYS) {
  test(`every compiled module has one pinned ${hostKey} asset`, () => {
    const source = readRegistrySource();
    const assets = bundledAssets(source, hostKey);
    // Every registry module, including TinyComputer v0.11, publishes these targets.
    assert.equal(assets.length, parseAllList(source).length);
    assert.ok(assets.some((a) => a.id === "tinycomputer"));
    assert.equal(new Set(assets.map((asset) => asset.id)).size, assets.length);
    for (const asset of assets) {
      assert.equal(asset.hostKey, hostKey);
      assert.match(asset.url, /^https:\/\/github\.com\/(?:tinyhumansai|senamakel)\//);
      assert.ok(asset.archive.includes(`-${hostKey}.`));
      assert.match(asset.sha256, /^[a-f0-9]{64}$/);
    }
  });
}

test("a missing pin fails staging before any download", () => {
  const source = readRegistrySource().replace(
    'host_key: "windows-2022-x86_64"',
    'host_key: "removed-windows-asset"',
  );
  assert.throws(
    () => bundledAssets(source, "windows-2022-x86_64"),
    /has no windows-2022-x86_64 asset/,
  );
});

test("the default host key is the oldest published build for each platform", () => {
  assert.equal(defaultHostKey("darwin", "arm64"), "macos-15-arm64");
  assert.equal(defaultHostKey("darwin", "x64"), "macos-15-x86_64");
  assert.equal(defaultHostKey("linux", "x64"), "ubuntu-22.04-x86_64");
  assert.equal(defaultHostKey("linux", "arm64"), "ubuntu-22.04-arm64");
  assert.equal(defaultHostKey("win32", "x64"), "windows-2022-x86_64");
  assert.equal(defaultHostKey("win32", "arm64"), "windows-11-arm64");
  assert.throws(() => defaultHostKey("freebsd", "x64"), /no bundled modules/);
});

test("Windows extraction rejects an archive entry escaping its destination", {
  skip: process.platform !== "win32",
}, () => {
  const root = mkdtempSync(join(tmpdir(), "openhuman-zip-boundary-"));
  const destination = join(root, "modules");
  mkdirSync(destination);
  const archive = join(root, "escape.zip");
  const name = Buffer.from("../escape.txt");
  const local = Buffer.alloc(30);
  local.writeUInt32LE(0x04034b50, 0);
  local.writeUInt16LE(20, 4);
  local.writeUInt16LE(name.length, 26);
  const central = Buffer.alloc(46);
  central.writeUInt32LE(0x02014b50, 0);
  central.writeUInt16LE(20, 4);
  central.writeUInt16LE(20, 6);
  central.writeUInt16LE(name.length, 28);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(1, 8);
  end.writeUInt16LE(1, 10);
  end.writeUInt32LE(central.length + name.length, 12);
  end.writeUInt32LE(local.length + name.length, 16);
  writeFileSync(archive, Buffer.concat([local, name, central, name, end]));

  assert.throws(
    () => extractWindowsZip(archive, destination),
    (error) => /outside/i.test(error.stderr?.toString() ?? ""),
  );
  assert.throws(() => readFileSync(join(root, "escape.txt")), { code: "ENOENT" });
});

test("host keys follow the Rust target triple, not the runner", () => {
  assert.equal(hostKeyForTarget("aarch64-apple-darwin"), "macos-15-arm64");
  assert.equal(hostKeyForTarget("x86_64-apple-darwin"), "macos-15-x86_64");
  assert.equal(hostKeyForTarget("x86_64-unknown-linux-gnu"), "ubuntu-22.04-x86_64");
  assert.equal(hostKeyForTarget("aarch64-unknown-linux-gnu"), "ubuntu-22.04-arm64");
  assert.equal(hostKeyForTarget("x86_64-pc-windows-msvc"), "windows-2022-x86_64");
  assert.equal(hostKeyForTarget("aarch64-pc-windows-msvc"), "windows-11-arm64");
  assert.throws(() => hostKeyForTarget("wasm32-unknown-unknown"), /no bundled modules/);
});

test("only macOS bundles replace the archive with its digest marker", () => {
  for (const hostKey of HOST_KEYS) {
    assert.equal(keepsArchive(hostKey), !hostKey.startsWith("macos-"), hostKey);
  }
});

test("a macOS entry keeps only the archive's verified digest, in tinybus's marker format", () => {
  const dir = mkdtempSync(join(tmpdir(), "openhuman-marker-"));
  const archive = join(dir, "demo-1.0.0-macos-15-arm64.tar.gz");
  writeFileSync(archive, "archive bytes");
  writeFileSync(join(dir, "libdemo.dylib"), "library bytes");
  const sha = createHash("sha256").update(readFileSync(archive)).digest("hex");

  replaceArchiveWithMarker(archive, sha);

  assert.equal(existsSync(archive), false, "the archive must not ship");
  // tinybus reads `<archive>.sha256`, trims it and compares it to the pin.
  assert.equal(readFileSync(`${archive}.sha256`, "utf8"), `${sha}\n`);
  assert.equal(readFileSync(join(dir, "libdemo.dylib"), "utf8"), "library bytes");
});

async function withServer(handler, run) {
  const server = createServer(handler);
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  try {
    await run(`http://127.0.0.1:${server.address().port}/a`);
  } finally {
    server.closeAllConnections();
    await new Promise((resolve) => server.close(resolve));
  }
}

test("download keeps the archive bytes and refuses a content-encoded body", async () => {
  const dir = mkdtempSync(join(tmpdir(), "openhuman-download-"));
  const out = join(dir, "a.bin");
  const raw = Buffer.from("raw archive bytes");
  await withServer((req, res) => res.end(raw), async (url) => {
    await download(url, out, 5000);
    assert.deepEqual(readFileSync(out), raw);
  });
  await withServer(
    (req, res) => {
      res.setHeader("content-encoding", "gzip");
      res.end(gzipSync(raw));
    },
    async (url) => {
      await assert.rejects(download(url, out, 5000), /Content-Encoding gzip/);
    },
  );
});

test("a stalled download times out instead of hanging", async () => {
  const dir = mkdtempSync(join(tmpdir(), "openhuman-download-"));
  await withServer(
    (req, res) => {
      res.write("partial");
    },
    async (url) => {
      await assert.rejects(download(url, join(dir, "a.bin"), 200), /download of/);
    },
  );
});

for (const hostKey of ["macos-15-arm64", "ubuntu-22.04-x86_64"]) {
  test(`stageModules on ${hostKey} ${keepsArchive(hostKey) ? "keeps the archive" : "replaces the archive with its verified digest"}`, async () => {
    const root = mkdtempSync(join(tmpdir(), "openhuman-stage-"));
    const source = join(root, "src");
    mkdirSync(source);
    const library = hostKey.startsWith("macos-") ? "libdemo.dylib" : "libdemo.so";
    writeFileSync(join(source, library), "library bytes");
    const archiveName = `demo-1.0.0-${hostKey}.tar.gz`;
    const built = join(root, archiveName);
    execFileSync("tar", ["-czf", built, "-C", source, library]);
    const bytes = readFileSync(built);
    const sha256 = createHash("sha256").update(bytes).digest("hex");
    const output = join(root, "out");

    await withServer((req, res) => res.end(bytes), async (url) => {
      await stageModules({
        hostKey,
        output,
        assets: [{ id: "demo", version: "1.0.0", hostKey, archive: archiveName, url, sha256 }],
      });
    });

    const dir = join(output, "demo", "1.0.0", hostKey);
    assert.equal(readFileSync(join(dir, library), "utf8"), "library bytes");
    if (keepsArchive(hostKey)) {
      assert.deepEqual(readFileSync(join(dir, archiveName)), bytes);
      assert.equal(existsSync(join(dir, `${archiveName}.sha256`)), false);
    } else {
      assert.equal(existsSync(join(dir, archiveName)), false, "the archive must not ship");
      assert.equal(readFileSync(join(dir, `${archiveName}.sha256`), "utf8"), `${sha256}\n`);
    }
  });
}

/** Every path under `root` (not following symlinks) with its mode bits. */
function modesUnder(root) {
  return readdirSync(root, { withFileTypes: true }).flatMap((entry) => {
    const path = join(root, entry.name);
    const mode = lstatSync(path).mode & 0o7777;
    return entry.isDirectory()
      ? [{ path, mode, dir: true }, ...modesUnder(path)]
      : [{ path, mode, dir: false }];
  });
}

// tinybus refuses a module whose directory, or any ancestor of it, another
// account can write ("module directory is writable by another user"). The
// staged tree ships inside the AppImage/deb as-is, so it must not carry the
// build host's umask or the release tarball's mode bits.
test("staged modules carry no group or other write bits, whatever the umask and archive modes", {
  skip: process.platform === "win32",
}, async () => {
  const hostKey = "ubuntu-22.04-x86_64";
  const root = mkdtempSync(join(tmpdir(), "openhuman-stage-modes-"));
  const source = join(root, "src");
  mkdirSync(join(source, "lib"), { recursive: true });
  const library = "lib/libdemo.so";
  writeFileSync(join(source, library), "library bytes");
  writeFileSync(join(source, "README"), "notes");
  chmodSync(join(source, "lib"), 0o777);
  chmodSync(join(source, library), 0o777);
  chmodSync(join(source, "README"), 0o666);
  const archiveName = `demo-1.0.0-${hostKey}.tar.gz`;
  const built = join(root, archiveName);
  execFileSync("tar", ["-czf", built, "-C", source, "lib", "README"]);
  const bytes = readFileSync(built);
  const sha256 = createHash("sha256").update(bytes).digest("hex");
  const output = join(root, "out");

  // Ubuntu's user-private-group default.
  const previousUmask = process.umask(0o002);
  try {
    await withServer((req, res) => res.end(bytes), async (url) => {
      await stageModules({
        hostKey,
        output,
        assets: [{ id: "demo", version: "1.0.0", hostKey, archive: archiveName, url, sha256 }],
      });
    });
  } finally {
    process.umask(previousUmask);
  }

  const entries = [
    { path: output, mode: lstatSync(output).mode & 0o7777, dir: true },
    ...modesUnder(output),
  ];
  assert.ok(entries.some((e) => e.path.endsWith("libdemo.so")), "the library was staged");
  for (const { path, mode, dir } of entries) {
    assert.equal(mode, dir ? 0o755 : 0o644, `${path} is ${mode.toString(8)}`);
  }
});

test("normalising staged permissions never follows a symlink out of the tree", {
  skip: process.platform === "win32",
}, () => {
  const root = mkdtempSync(join(tmpdir(), "openhuman-stage-symlink-"));
  const outside = join(root, "outside");
  writeFileSync(outside, "not staged");
  chmodSync(outside, 0o600);
  const staged = join(root, "staged");
  mkdirSync(staged);
  symlinkSync(outside, join(staged, "link"));

  normalizeStagedPermissions(staged);

  assert.equal(lstatSync(outside).mode & 0o7777, 0o600);
  assert.equal(lstatSync(staged).mode & 0o7777, 0o755);
});

// Windows app-local VC runtime staging — issue #8499 / Sentry TAURI-RUST-14J5.
//
// `isWindowsHostKey`, `findWindowsVcRuntimeDir`, and `stageWindowsAppLocalRuntime`
// close the "fresh Windows 10/11 without VC redist installed" load failure for
// every tiny* native module by shipping vcruntime140.dll beside the module
// library. The tinybus loader uses LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR |
// LOAD_LIBRARY_SEARCH_SYSTEM32, which never searches PATH or sibling cache
// dirs, so an app-local copy is the only reliable resolution path.
test("isWindowsHostKey picks exactly the windows-* host keys", () => {
  assert.equal(isWindowsHostKey("windows-2022-x86_64"), true);
  assert.equal(isWindowsHostKey("windows-11-arm64"), true);
  assert.equal(isWindowsHostKey("windows-2025-x86_64"), true);
  assert.equal(isWindowsHostKey("ubuntu-22.04-x86_64"), false);
  assert.equal(isWindowsHostKey("macos-15-arm64"), false);
  assert.equal(isWindowsHostKey("fedora-43-x86_64"), false);
});

test("findWindowsVcRuntimeDir reads OPENHUMAN_VC_REDIST_DIR when it has the DLL", () => {
  const dir = mkdtempSync(join(tmpdir(), "openhuman-vcrt-override-"));
  writeFileSync(join(dir, "vcruntime140.dll"), "stub vc runtime bytes");
  const previous = process.env.OPENHUMAN_VC_REDIST_DIR;
  process.env.OPENHUMAN_VC_REDIST_DIR = dir;
  try {
    assert.equal(findWindowsVcRuntimeDir("windows-2022-x86_64"), dir);
    assert.equal(findWindowsVcRuntimeDir("windows-11-arm64"), dir);
  } finally {
    if (previous === undefined) delete process.env.OPENHUMAN_VC_REDIST_DIR;
    else process.env.OPENHUMAN_VC_REDIST_DIR = previous;
  }
});

test("findWindowsVcRuntimeDir refuses an OPENHUMAN_VC_REDIST_DIR without vcruntime140.dll", () => {
  const dir = mkdtempSync(join(tmpdir(), "openhuman-vcrt-empty-"));
  const previous = process.env.OPENHUMAN_VC_REDIST_DIR;
  process.env.OPENHUMAN_VC_REDIST_DIR = dir;
  try {
    assert.throws(
      () => findWindowsVcRuntimeDir("windows-2022-x86_64"),
      /does not contain vcruntime140\.dll/,
    );
  } finally {
    if (previous === undefined) delete process.env.OPENHUMAN_VC_REDIST_DIR;
    else process.env.OPENHUMAN_VC_REDIST_DIR = previous;
  }
});

test("findWindowsVcRuntimeDir refuses a non-Windows host_key as a programming error", () => {
  assert.throws(
    () => findWindowsVcRuntimeDir("ubuntu-22.04-x86_64"),
    /non-Windows host_key/,
  );
});

test("findWindowsVcRuntimeDir fails actionably when no source is available on a non-Windows build host", {
  skip: process.platform === "win32",
}, () => {
  const previous = process.env.OPENHUMAN_VC_REDIST_DIR;
  const previousTools = process.env.VCToolsRedistDir;
  delete process.env.OPENHUMAN_VC_REDIST_DIR;
  delete process.env.VCToolsRedistDir;
  try {
    assert.throws(
      () => findWindowsVcRuntimeDir("windows-2022-x86_64"),
      /OPENHUMAN_VC_REDIST_DIR/,
    );
  } finally {
    if (previous !== undefined) process.env.OPENHUMAN_VC_REDIST_DIR = previous;
    if (previousTools !== undefined) process.env.VCToolsRedistDir = previousTools;
  }
});

test("stageWindowsAppLocalRuntime is a no-op on non-Windows host keys", () => {
  const target = mkdtempSync(join(tmpdir(), "openhuman-vcrt-noop-"));
  const copied = stageWindowsAppLocalRuntime(target, "ubuntu-22.04-x86_64");
  assert.deepEqual(copied, []);
  assert.deepEqual(readdirSync(target), []);
});

/** Common env setup: use a controlled VC runtime source and skip the
 * platform-dependent Authenticode check so stub PE bytes are admitted.
 * Handles both sync and async `run` callbacks (restores env on both paths). */
function withVcRuntimeSource(source, run) {
  const previousDir = process.env.OPENHUMAN_VC_REDIST_DIR;
  const previousSkip = process.env.OPENHUMAN_SKIP_AUTHENTICODE_CHECK;
  process.env.OPENHUMAN_VC_REDIST_DIR = source;
  process.env.OPENHUMAN_SKIP_AUTHENTICODE_CHECK = "1";
  const restore = () => {
    if (previousDir === undefined) delete process.env.OPENHUMAN_VC_REDIST_DIR;
    else process.env.OPENHUMAN_VC_REDIST_DIR = previousDir;
    if (previousSkip === undefined) delete process.env.OPENHUMAN_SKIP_AUTHENTICODE_CHECK;
    else process.env.OPENHUMAN_SKIP_AUTHENTICODE_CHECK = previousSkip;
  };
  try {
    const result = run();
    if (result && typeof result.then === "function") {
      return result.finally(restore);
    }
    restore();
    return result;
  } catch (err) {
    restore();
    throw err;
  }
}

test("peMachine reads x64 and arm64 PE machine fields", () => {
  const dir = mkdtempSync(join(tmpdir(), "openhuman-pe-"));
  const x64 = join(dir, "x64.dll");
  const arm64 = join(dir, "arm64.dll");
  writeFileSync(x64, PE_X64);
  writeFileSync(arm64, PE_ARM64);
  assert.equal(peMachine(x64), 0x8664);
  assert.equal(peMachine(arm64), 0xaa64);
});

test("peMachine refuses a file without an MZ signature", () => {
  const dir = mkdtempSync(join(tmpdir(), "openhuman-pe-bad-"));
  const notPe = join(dir, "not-a-pe.dll");
  // Must be >= 0x40 bytes so the signature check (not the size check) fails.
  writeFileSync(notPe, Buffer.alloc(0x100, "a"));
  assert.throws(() => peMachine(notPe), /missing MZ signature/);
});

test("stageWindowsAppLocalRuntime copies the Windows runtime next to the module library", () => {
  const source = mkdtempSync(join(tmpdir(), "openhuman-vcrt-src-"));
  writeFileSync(join(source, "vcruntime140.dll"), PE_X64);
  const target = mkdtempSync(join(tmpdir(), "openhuman-vcrt-target-"));
  writeFileSync(join(target, "tinyconnectors.dll"), "stub module bytes");

  withVcRuntimeSource(source, () => {
    const copied = stageWindowsAppLocalRuntime(target, "windows-2022-x86_64");
    assert.deepEqual(copied, [join(target, "vcruntime140.dll")]);
    assert.equal(peMachine(join(target, "vcruntime140.dll")), 0x8664);
    assert.equal(
      readFileSync(join(target, "tinyconnectors.dll"), "utf8"),
      "stub module bytes",
    );
  });
});

test("stageWindowsAppLocalRuntime refuses an x64 VC runtime for an ARM64 host_key", () => {
  const source = mkdtempSync(join(tmpdir(), "openhuman-vcrt-mismatch-"));
  writeFileSync(join(source, "vcruntime140.dll"), PE_X64);
  const target = mkdtempSync(join(tmpdir(), "openhuman-vcrt-mismatch-target-"));

  withVcRuntimeSource(source, () => {
    assert.throws(
      () => stageWindowsAppLocalRuntime(target, "windows-11-arm64"),
      /PE machine 0x8664 does not match windows-11-arm64/,
    );
    assert.equal(
      existsSync(join(target, "vcruntime140.dll")),
      false,
      "a mismatched runtime must not be left behind in the target dir",
    );
  });
});

test("stageWindowsAppLocalRuntime refuses a non-PE source file", () => {
  const source = mkdtempSync(join(tmpdir(), "openhuman-vcrt-non-pe-"));
  // >= 0x40 bytes so the signature check runs (not the size check).
  writeFileSync(join(source, "vcruntime140.dll"), Buffer.alloc(0x100, "a"));
  const target = mkdtempSync(join(tmpdir(), "openhuman-vcrt-non-pe-target-"));

  withVcRuntimeSource(source, () => {
    assert.throws(
      () => stageWindowsAppLocalRuntime(target, "windows-2022-x86_64"),
      /missing MZ signature/,
    );
  });
});

/**
 * Build a minimal valid ZIP containing one entry at `entryPath` with the
 * given uncompressed bytes. Hand-rolled so these tests do not depend on a
 * `zip` binary being on the runner. The CRC-32 of `payload` is written into
 * both header slots — `unzip` on Linux/macOS verifies it and rejects a zero
 * CRC, while Windows `System.IO.Compression.ZipFile` tolerates it (which is
 * why an earlier revision passed on win32 and failed on every other CI leg).
 */
function makeSingleEntryZip(entryPath, payload) {
  const name = Buffer.from(entryPath);
  const payloadCrc = crc32(payload);
  const localHeader = Buffer.alloc(30);
  localHeader.writeUInt32LE(0x04034b50, 0);
  localHeader.writeUInt16LE(20, 4);
  localHeader.writeUInt16LE(0, 6);
  localHeader.writeUInt16LE(0, 8);
  localHeader.writeUInt32LE(payloadCrc, 14);
  localHeader.writeUInt32LE(payload.length, 18);
  localHeader.writeUInt32LE(payload.length, 22);
  localHeader.writeUInt16LE(name.length, 26);
  const central = Buffer.alloc(46);
  central.writeUInt32LE(0x02014b50, 0);
  central.writeUInt16LE(20, 4);
  central.writeUInt16LE(20, 6);
  central.writeUInt32LE(payloadCrc, 16);
  central.writeUInt32LE(payload.length, 20);
  central.writeUInt32LE(payload.length, 24);
  central.writeUInt16LE(name.length, 28);
  const localSize = localHeader.length + name.length + payload.length;
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(1, 8);
  end.writeUInt16LE(1, 10);
  end.writeUInt32LE(central.length + name.length, 12);
  end.writeUInt32LE(localSize, 16);
  return Buffer.concat([localHeader, name, payload, central, name, end]);
}

// stageModules asserts exactly one native library per archive. The staged VC
// runtime files are also `.dll`s; they must not count toward that invariant or
// every Windows module would fail the check.
test("stageModules on windows-2022-x86_64 ships vcruntime140.dll beside the module", async () => {
  const hostKey = "windows-2022-x86_64";
  const root = mkdtempSync(join(tmpdir(), "openhuman-stage-windows-"));
  const library = "demo.dll";
  const archiveName = `demo-1.0.0-${hostKey}.zip`;
  const built = join(root, archiveName);
  const archiveBytes = makeSingleEntryZip(library, Buffer.from("library bytes"));
  writeFileSync(built, archiveBytes);
  const sha256 = createHash("sha256").update(archiveBytes).digest("hex");
  const output = join(root, "out");

  const redist = mkdtempSync(join(tmpdir(), "openhuman-vcrt-source-"));
  writeFileSync(join(redist, "vcruntime140.dll"), PE_X64);

  await withVcRuntimeSource(redist, async () => {
    await withServer((req, res) => res.end(archiveBytes), async (url) => {
      await stageModules({
        hostKey,
        output,
        assets: [{ id: "demo", version: "1.0.0", hostKey, archive: archiveName, url, sha256 }],
      });
    });
  });

  const dir = join(output, "demo", "1.0.0", hostKey);
  // Module library, archive, and VC runtime all land in the host_key dir.
  assert.ok(existsSync(join(dir, library)), "the module library was staged");
  assert.ok(existsSync(join(dir, archiveName)), "the archive is kept on Windows");
  assert.equal(
    peMachine(join(dir, "vcruntime140.dll")),
    0x8664,
    "vcruntime140.dll is staged beside the module and admits as x64",
  );
});

// If an archive puts its library in a subdirectory, the VC runtime must land
// in that same subdirectory — the tinybus loader uses DLL_LOAD_DIR, which
// resolves dependencies only in the module's own directory, never an ancestor.
// Regression for review feedback on PR #8513 (CodeRabbit, 2026-10-30).
test("stageModules on windows-2022-x86_64 places vcruntime140.dll beside a library in a subdirectory", async () => {
  const hostKey = "windows-2022-x86_64";
  const root = mkdtempSync(join(tmpdir(), "openhuman-stage-windows-sub-"));
  const library = "nested/demo.dll";
  const archiveName = `demo-1.0.0-${hostKey}.zip`;
  const built = join(root, archiveName);
  const archiveBytes = makeSingleEntryZip(library, Buffer.from("library bytes"));
  writeFileSync(built, archiveBytes);
  const sha256 = createHash("sha256").update(archiveBytes).digest("hex");
  const output = join(root, "out");

  const redist = mkdtempSync(join(tmpdir(), "openhuman-vcrt-source-sub-"));
  writeFileSync(join(redist, "vcruntime140.dll"), PE_X64);

  await withVcRuntimeSource(redist, async () => {
    await withServer((req, res) => res.end(archiveBytes), async (url) => {
      await stageModules({
        hostKey,
        output,
        assets: [{ id: "demo", version: "1.0.0", hostKey, archive: archiveName, url, sha256 }],
      });
    });
  });

  const dir = join(output, "demo", "1.0.0", hostKey);
  const nested = join(dir, "nested");
  assert.ok(existsSync(join(nested, "demo.dll")), "the subdir-nested library was staged");
  assert.ok(
    existsSync(join(nested, "vcruntime140.dll")),
    "vcruntime140.dll is staged in the subdir with the module, not at the host_key root",
  );
  assert.equal(
    existsSync(join(dir, "vcruntime140.dll")),
    false,
    "no stray runtime at the host_key root (where the loader would not find it)",
  );
});
