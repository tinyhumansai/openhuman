#!/usr/bin/env node
// The test runner builds this fixture from the pinned submodule. Record that
// artifact's digest as operator trust; TinyBus still verifies it before loading
// and refuses confidential delivery when the file or allowlist does not match.
import { createHash } from "node:crypto";
import { basename, join, resolve } from "node:path";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";

const [source, destination] = process.argv.slice(2);
if (!source || !destination) {
  throw new Error(
    "usage: stage-connector-test-module.mjs <built-library> <fixture-directory>",
  );
}
const bytes = readFileSync(source);
const filename = basename(source);
if (!/^libtinyconnectors\.(so|dylib)$|^tinyconnectors\.dll$/.test(filename)) {
  throw new Error("expected the locally built TinyConnectors library");
}
const directory = resolve(destination);
mkdirSync(directory, { recursive: true });
const artifact = join(directory, filename);
writeFileSync(artifact, bytes);
const digest = createHash("sha256").update(bytes).digest("hex");
writeFileSync(join(directory, "modules.toml"), `"${filename}" = "${digest}"\n`);
process.stdout.write(`${artifact}\n`);
