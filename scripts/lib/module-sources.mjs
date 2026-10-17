// Pure helpers for scripts/modules/build.mjs (tinyhumansai/openhuman#7613).
//
// The build recipe for each module lives in its registry record, as the
// `source: SourceSpec { .. }` field (crates/openhuman-core/src/modules/types.rs).
// This file reads those records straight from the Rust source, the same way
// scripts/lib/module-pins.mjs reads the pins, so there is no second table and no
// generated JSON that can go stale.
//
// Nothing here writes a registry pin: a locally built digest is operator trust
// (a `modules.toml` beside the artifact), never a registry digest.

import { createHash } from "node:crypto";

/** Shell/env variable that selects `source` for one module id. */
export function moduleSourceEnv(id) {
  return `OPENHUMAN_MODULE_SOURCE_${id.toUpperCase().replace(/[^A-Z0-9]/g, "_")}`;
}

/**
 * Every record's id and SourceSpec from the registry source text.
 * Returns `Map<id, { id, submodule, manifest, package, lib_name, features }>`.
 */
export function parseSourceSpecs(src) {
  const specs = new Map();
  for (const chunk of src.split(/\bModuleRecord \{/).slice(1)) {
    const id = chunk.match(/\bid: "([^"]+)"/)?.[1];
    const block = chunk.match(/\bsource: SourceSpec \{([\s\S]*?)\n\s*\},/)?.[1];
    if (!id || !block) continue;
    const field = (name) => block.match(new RegExp(`${name}: "([^"]*)"`))?.[1];
    const features = block.match(/features: &\[([^\]]*)\]/)?.[1] ?? "";
    const spec = {
      id,
      submodule: field("submodule"),
      manifest: field("manifest"),
      package: field("package"),
      lib_name: field("lib_name"),
      features: [...features.matchAll(/"([^"]+)"/g)].map((m) => m[1]),
    };
    for (const key of ["submodule", "manifest", "package", "lib_name"]) {
      if (!spec[key]) throw new Error(`${id}: SourceSpec has no ${key}`);
    }
    specs.set(id, spec);
  }
  return specs;
}

/** The file name Cargo gives the cdylib for `spec` on `platform` (node's). */
export function artifactFilename(spec, platform = process.platform) {
  if (platform === "win32") return `${spec.lib_name}.dll`;
  if (platform === "darwin") return `lib${spec.lib_name}.dylib`;
  return `lib${spec.lib_name}.so`;
}

/**
 * The cache key for one build: the submodule's tree (plus any uncommitted
 * change), the toolchain, the target, the profile and the build recipe. Any of
 * them changing yields a different key, so a stale artifact is never reused.
 */
export function cacheKey({
  tree,
  dirty = "",
  rustc,
  triple,
  profile,
  features = [],
  spec,
}) {
  const parts = [
    `tree=${tree}`,
    `dirty=${dirty}`,
    `rustc=${rustc}`,
    `triple=${triple}`,
    `profile=${profile}`,
    `features=${[...features].sort().join(",")}`,
    `package=${spec.package}`,
    `manifest=${spec.manifest}`,
  ];
  return createHash("sha256").update(parts.join("\n")).digest("hex");
}

/** Lowercase hex SHA-256 of `bytes`. */
export function sha256Hex(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

/**
 * The operator-trust `modules.toml` body for one built artifact: its file name
 * mapped to the SHA-256 of its bytes. TinyBus still verifies the file against
 * this on load and refuses confidential delivery when they differ.
 */
export function trustToml(filename, bytes) {
  return `"${filename}" = "${sha256Hex(bytes)}"\n`;
}

/** Single-quote `value` for a POSIX shell. */
export function shellQuote(value) {
  return `'${String(value).replace(/'/g, `'\\''`)}'`;
}
