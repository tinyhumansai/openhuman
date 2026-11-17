import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const script = path.join(repoRoot, "scripts/ci/self-hosted/prune-coverage-binaries.sh");
const hash = "0123456789abcdef";

test("prune removes instrumented test executables and profiles but keeps build inputs", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "prune-coverage-"));
  try {
    const deps = path.join(dir, "llvm-cov-target/debug/deps");
    const build = path.join(dir, "llvm-cov-target/debug/build", `serde-${hash}`);
    fs.mkdirSync(deps, { recursive: true });
    fs.mkdirSync(build, { recursive: true });
    const put = (file, mode = 0o644) => {
      fs.writeFileSync(file, "x");
      fs.chmodSync(file, mode);
      return file;
    };
    const removed = [
      put(path.join(deps, `json_rpc_e2e-${hash}`), 0o755),
      put(path.join(deps, `openhuman-${hash}`), 0o755),
      put(path.join(dir, "llvm-cov-target", `openhuman-1-${hash}_0.profraw`)),
      put(path.join(dir, "llvm-cov-target", "openhuman.profdata")),
    ];
    const kept = [
      put(path.join(deps, `libopenhuman-${hash}.rlib`)),
      put(path.join(deps, `libopenhuman-${hash}.rmeta`)),
      put(path.join(deps, `libserde_derive-${hash}.so`), 0o755),
      put(path.join(deps, `openhuman-${hash}.d`)),
      // A hash-named file that is not executable is not a test binary.
      put(path.join(deps, `notes-${hash}`)),
      put(path.join(build, `build_script_build-${hash}`), 0o755),
    ];
    const out = execFileSync("bash", [script, dir], { encoding: "utf8" });
    for (const file of removed) assert.equal(fs.existsSync(file), false, file);
    for (const file of kept) assert.equal(fs.existsSync(file), true, file);
    assert.match(out, /removed 2 test executables and 2 profiles/);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("prune is a no-op without a coverage target", () => {
  const out = execFileSync("bash", [script, path.join(os.tmpdir(), "no-such-coverage-target")], {
    encoding: "utf8",
  });
  assert.match(out, /nothing to prune/);
});
