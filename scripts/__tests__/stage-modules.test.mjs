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
import { gzipSync } from "node:zlib";
import { test } from "node:test";

import {
  parseReleaseUrls,
  readRegistrySource,
} from "../ci/self-hosted/test-module-assets.mjs";
import { parseAllList } from "../lib/module-pins.mjs";
import {
  bundledAssets,
  defaultHostKey,
  download,
  hostKeyForTarget,
  extractWindowsZip,
  keepsArchive,
  normalizeStagedPermissions,
  replaceArchiveWithMarker,
  stageModules,
} from "../release/stage-modules.mjs";

const HOST_KEYS = [
  "macos-15-arm64",
  "macos-15-x86_64",
  "ubuntu-22.04-x86_64",
  "ubuntu-22.04-arm64",
  "windows-2022-x86_64",
  "windows-11-arm64",
];

test("an unpublished module cannot borrow the next record's release URL", () => {
  const source = `const PENDING: ModuleRecord = ModuleRecord {
    id: "pending",
    version: "1.0.0",
    release_url: "",
    assets: &[],
};
const RELEASED: ModuleRecord = ModuleRecord {
    id: "released",
    version: "2.0.0",
    release_url: "https://github.com/example/released/releases/tag/v2.0.0",
    assets: &[],
};`;
  const urls = parseReleaseUrls(source);
  assert.equal(urls.has("pending"), false);
  assert.equal(
    urls.get("released"),
    "https://github.com/example/released/releases/tag/v2.0.0",
  );
});

for (const hostKey of HOST_KEYS) {
  test(`every compiled module has one pinned ${hostKey} asset`, () => {
    const source = readRegistrySource();
    const assets = bundledAssets(source, hostKey);
    const linux = hostKey.startsWith("ubuntu-");
    // tinycomputer publishes no Linux build; every other module is bundled.
    assert.equal(assets.length, parseAllList(source).length - (linux ? 1 : 0));
    assert.equal(assets.some((a) => a.id === "tinycomputer"), !linux);
    assert.equal(new Set(assets.map((asset) => asset.id)).size, assets.length);
    for (const asset of assets) {
      assert.equal(asset.hostKey, hostKey);
      assert.match(asset.url, /^https:\/\/github\.com\/tinyhumansai\//);
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
