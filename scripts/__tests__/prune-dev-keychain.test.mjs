import { execFileSync } from "node:child_process";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const SCRIPT = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
  "prune-dev-keychain.mjs",
);

/** A keychain holding two dead temp-dir entries and two entries worth keeping. */
function sampleKeychain() {
  return {
    ".tmpZ0Fsqb:auth:app-session:default": "leaked-one",
    ".tmpPX0lzc:auth:openai:prod": "leaked-two",
    "local-dragonfly:auth:app-session:default": "real-session",
    "user-5307:auth:app-session:default": "real-user",
  };
}

function writeKeychain(contents) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "prune-dev-keychain-"));
  const file = path.join(dir, "dev-keychain.json");
  fs.writeFileSync(
    file,
    typeof contents === "string" ? contents : JSON.stringify(contents),
  );
  return file;
}

function run(args, { expectFailure = false } = {}) {
  try {
    return execFileSync("node", [SCRIPT, ...args], {
      encoding: "utf8",
      stdio: "pipe",
    });
  } catch (error) {
    if (!expectFailure) throw error;
    return `${error.stdout ?? ""}${error.stderr ?? ""}`;
  }
}

test("dry run reports leaked entries and writes nothing", () => {
  const file = writeKeychain(sampleKeychain());
  const before = fs.readFileSync(file, "utf8");

  const output = run(["--file", file]);

  assert.match(output, /Leaked:\s+2/);
  assert.match(output, /Kept:\s+2/);
  assert.match(output, /keep {2}local-dragonfly:auth:app-session:default/);
  assert.match(output, /Dry run/);
  assert.equal(
    fs.readFileSync(file, "utf8"),
    before,
    "dry run must not modify the file",
  );
});

test("--apply removes only temp-dir-keyed entries and backs the file up", () => {
  const file = writeKeychain(sampleKeychain());

  run(["--file", file, "--apply"]);

  const pruned = JSON.parse(fs.readFileSync(file, "utf8"));
  assert.deepEqual(Object.keys(pruned).sort(), [
    "local-dragonfly:auth:app-session:default",
    "user-5307:auth:app-session:default",
  ]);
  assert.equal(
    pruned["local-dragonfly:auth:app-session:default"],
    "real-session",
  );

  const backups = fs
    .readdirSync(path.dirname(file))
    .filter((name) => name.startsWith("dev-keychain.json.backup-"));
  assert.equal(backups.length, 1, "exactly one backup should be written");
  assert.deepEqual(
    JSON.parse(
      fs.readFileSync(path.join(path.dirname(file), backups[0]), "utf8"),
    ),
    sampleKeychain(),
    "the backup must hold the original contents",
  );
});

test("a keychain with nothing leaked is left alone", () => {
  const file = writeKeychain({
    "local-dragonfly:auth:app-session:default": "real-session",
  });
  const before = fs.readFileSync(file, "utf8");

  const output = run(["--file", file, "--apply"]);

  assert.match(output, /Nothing to prune/);
  assert.equal(fs.readFileSync(file, "utf8"), before);
});

test("invalid JSON is refused rather than rewritten", () => {
  const file = writeKeychain("{ not json");
  const before = fs.readFileSync(file, "utf8");

  const output = run(["--file", file, "--apply"], { expectFailure: true });

  assert.match(output, /not valid JSON/);
  assert.equal(fs.readFileSync(file, "utf8"), before);
});

test("a missing file is reported, not created", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "prune-dev-keychain-"));
  const file = path.join(dir, "dev-keychain.json");

  const output = run(["--file", file, "--apply"]);

  assert.match(output, /nothing to do/);
  assert.equal(fs.existsSync(file), false);
});
