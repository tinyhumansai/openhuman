import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const repoRoot = path.join(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const script = path.join(repoRoot, "scripts", "run-dev-macos.sh");

function runLauncher(env = {}) {
  const bin = fs.mkdtempSync(path.join(os.tmpdir(), "openhuman-fake-cargo-"));
  const cargo = path.join(bin, "cargo");
  fs.writeFileSync(cargo, "#!/usr/bin/env bash\nprintf 'identity=%s\\nport=%s\\nargs=%s\\n' \"$APPLE_SIGNING_IDENTITY\" \"$OPENHUMAN_DEV_PORT\" \"$*\"\n");
  fs.chmodSync(cargo, 0o755);
  return execFileSync("bash", [script], {
    cwd: repoRoot,
    encoding: "utf8",
    env: { ...process.env, ...env, PATH: `${bin}:${process.env.PATH}` },
  });
}

test("starts without an untracked .env using the default port", () => {
  const output = runLauncher({
    APPLE_SIGNING_IDENTITY: "",
    OPENHUMAN_DEV_PORT: "",
  });

  assert.match(output, /identity=OpenHuman Dev Signer/);
  assert.match(output, /port=1420/);
  assert.match(output, /devUrl.*localhost:1420/);
});

test("falls back to the default port when OPENHUMAN_DEV_PORT is invalid", () => {
  const output = runLauncher({ OPENHUMAN_DEV_PORT: "not-a-port" });

  assert.match(output, /port=1420/);
  assert.match(output, /devUrl.*localhost:1420/);
});

test("passes a validated custom port to Tauri and Vite", () => {
  const output = runLauncher({ OPENHUMAN_DEV_PORT: " 15321 " });

  assert.match(output, /port=15321/);
  assert.match(output, /devUrl.*localhost:15321/);
});

test("normalizes invalid numeric ports before starting Tauri and Vite", () => {
  const output = runLauncher({ OPENHUMAN_DEV_PORT: "65536" });

  assert.match(output, /port=1420/);
  assert.match(output, /devUrl.*localhost:1420/);
});
