#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { chmodSync, copyFileSync, existsSync, mkdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const root = resolve(__dirname, "..");

function run(cmd, args, cwd = root) {
  const res = spawnSync(cmd, args, { cwd, stdio: "inherit", shell: false });
  if (res.status !== 0) {
    process.exit(res.status ?? 1);
  }
}

function rustHostTriple() {
  const res = spawnSync("rustc", ["-vV"], { cwd: root, encoding: "utf8" });
  if (res.status !== 0 || !res.stdout) {
    console.error("[core:stage] failed to query rustc host triple");
    process.exit(res.status ?? 1);
  }
  const line = res.stdout
    .split("\n")
    .map((s) => s.trim())
    .find((s) => s.startsWith("host:"));
  const triple = line?.replace(/^host:\s*/, "").trim();
  if (!triple) {
    console.error("[core:stage] rustc host triple missing");
    process.exit(1);
  }
  return triple;
}

function cargoTargetDir() {
  if (process.env.CARGO_TARGET_DIR) {
    // Resolve against the repo root so this path stays consistent
    // with the `cargo build` invocation below (which runs with
    // `cwd: root`). If the script were invoked from a different
    // working directory and `CARGO_TARGET_DIR` were relative, a
    // bare `resolve()` would anchor it to the wrong cwd and the
    // staged binary lookup would miss.
    return resolve(root, process.env.CARGO_TARGET_DIR);
  }
  const res = spawnSync(
    "cargo",
    ["metadata", "--format-version", "1", "--no-deps", "--manifest-path", "Cargo.toml"],
    { cwd: root, encoding: "utf8", shell: false, maxBuffer: 64 * 1024 * 1024 },
  );
  if (res.status === 0 && res.stdout) {
    try {
      const meta = JSON.parse(res.stdout);
      if (meta.target_directory) return resolve(meta.target_directory);
    } catch {
      // fall through to default
    }
  }
  return join(root, "target");
}

const triple = rustHostTriple();
const isWindows = process.platform === "win32";
const binName = isWindows ? "openhuman-core.exe" : "openhuman-core";

console.log(
  `[core:stage] Building openhuman-core standalone binary for ${triple}...`,
);
run("cargo", ["build", "--manifest-path", "Cargo.toml", "--bin", "openhuman-core"]);

const targetDir = cargoTargetDir();
const source = join(targetDir, "debug", binName);
if (!existsSync(source)) {
  console.error(`[core:stage] compiled binary not found: ${source}`);
  process.exit(1);
}

const outputDir = join(root, "app", "src-tauri", "binaries");
mkdirSync(outputDir, { recursive: true });
const sidecarName = isWindows
  ? `openhuman-core-${triple}.exe`
  : `openhuman-core-${triple}`;
const dest = join(outputDir, sidecarName);
copyFileSync(source, dest);
if (!isWindows) {
  chmodSync(dest, 0o755);
}

console.log(`[core:stage] Staged sidecar: ${dest}`);

// macOS: sign with a stable local dev certificate so macOS TCC uses certificate
// identity (stable across rebuilds) instead of binary content hash (changes
// every compile). Without this, each recompile breaks existing TCC grants.
if (process.platform === "darwin") {
  const DEV_IDENTITY = "OpenHuman Dev Signer";
  const check = spawnSync(
    "bash",
    ["-c", `security find-identity -v -p codesigning 2>/dev/null | grep "${DEV_IDENTITY}" || true`],
    { cwd: root, encoding: "utf8" },
  );
  if (check.stdout && check.stdout.includes(DEV_IDENTITY)) {
    const signResult = spawnSync("codesign", ["--force", "--sign", DEV_IDENTITY, "--timestamp=none", dest], { cwd: root, stdio: "inherit", shell: false });
    const isCI = process.env.CI === "true" || process.env.CI === "1";
    if (signResult.status === 0) {
      console.log(`[core:stage] Signed sidecar with "${DEV_IDENTITY}"`);
    } else if (isCI) {
      console.error(`[core:stage] Dev signing failed (status ${signResult.status}) in CI — aborting.`);
      process.exit(signResult.status ?? 1);
    } else {
      console.warn(`[core:stage] Dev signing failed (status ${signResult.status}), continuing without stable signing.`);
    }
  } else {
    console.warn(
      `[core:stage] Dev signing identity "${DEV_IDENTITY}" not found.\n` +
      `[core:stage] Run 'bash scripts/setup-dev-codesign.sh' once to enable stable TCC grants.\n` +
      `[core:stage] Without signing, macOS accessibility grants break on every recompile.`,
    );
  }
}
