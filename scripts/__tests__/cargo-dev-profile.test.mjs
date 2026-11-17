import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const MANIFESTS = ["Cargo.toml", "crates/openhuman-app/Cargo.toml"];

/** The body of one `[table]` in a manifest, up to the next table header. */
function table(manifest, header) {
  const text = readFileSync(join(root, manifest), "utf8");
  const start = text.indexOf(`\n${header}\n`);
  if (start < 0) return null;
  const body = text.slice(start + header.length + 2);
  const next = body.search(/^\[/m);
  return next < 0 ? body : body.slice(0, next);
}

// Module admission hashes the host executable once per process, and nextest
// runs every test in its own process: an unoptimised SHA-256 over a ~500 MB
// test binary cost ~10 s for each of ~900 module-backed tests.
test("both workspaces optimise sha2 in dev and test builds", () => {
  for (const manifest of MANIFESTS) {
    const body = table(manifest, "[profile.dev.package.sha2]");
    assert.ok(body, `${manifest} has no [profile.dev.package.sha2]`);
    assert.match(body, /^opt-level = 3$/m, manifest);
  }
});
