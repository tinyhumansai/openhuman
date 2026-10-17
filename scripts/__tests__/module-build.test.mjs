import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  chmodSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { main, readRegistry, DEFAULT_ROOT } from "../modules/build.mjs";
import {
  artifactFilename,
  cacheKey,
  moduleSourceEnv,
  parseSourceSpecs,
  sha256Hex,
  trustToml,
} from "../lib/module-sources.mjs";

const SPEC = {
  id: "tinyx",
  submodule: "tinyx",
  manifest: "crates/tinyx-module/Cargo.toml",
  package: "tinyx-module",
  lib_name: "tinyx_module",
  features: [],
};
const BASE = {
  tree: "a".repeat(40),
  dirty: "",
  rustc: "rustc 1.90.0\nhost: x86_64-unknown-linux-gnu\n",
  triple: "x86_64-unknown-linux-gnu",
  profile: "release",
  features: [],
  spec: SPEC,
};

test("the cache key is stable and changes with every input it names", () => {
  const key = cacheKey(BASE);
  assert.match(key, /^[a-f0-9]{64}$/);
  assert.equal(cacheKey({ ...BASE }), key);
  assert.equal(
    cacheKey({ ...BASE, features: ["b", "a"] }),
    cacheKey({ ...BASE, features: ["a", "b"] }),
    "feature order does not matter",
  );
  for (const change of [
    { tree: "b".repeat(40) },
    { dirty: "diff --git a/x b/x" },
    { rustc: "rustc 1.91.0\nhost: x86_64-unknown-linux-gnu\n" },
    { triple: "aarch64-apple-darwin" },
    { profile: "dev" },
    { features: ["extra"] },
    { spec: { ...SPEC, package: "other" } },
    { spec: { ...SPEC, manifest: "Cargo.toml" } },
  ]) {
    assert.notEqual(cacheKey({ ...BASE, ...change }), key, JSON.stringify(change));
  }
});

test("the operator-trust file maps the artifact name to the sha256 of its bytes", () => {
  const bytes = Buffer.from("built module bytes");
  assert.equal(
    trustToml("libtinyx_module.so", bytes),
    `"libtinyx_module.so" = "${sha256Hex(bytes)}"\n`,
  );
  assert.equal(artifactFilename(SPEC, "linux"), "libtinyx_module.so");
  assert.equal(artifactFilename(SPEC, "darwin"), "libtinyx_module.dylib");
  assert.equal(artifactFilename(SPEC, "win32"), "tinyx_module.dll");
  assert.equal(moduleSourceEnv("tinydocs"), "OPENHUMAN_MODULE_SOURCE_TINYDOCS");
});

test("every registry record carries a SourceSpec whose manifest exists in its submodule", () => {
  const specs = parseSourceSpecs(readRegistry(DEFAULT_ROOT));
  assert.equal(specs.size, 13);
  for (const spec of specs.values()) {
    assert.equal(spec.submodule, spec.id);
    const manifest = join(DEFAULT_ROOT, "vendor", spec.submodule, spec.manifest);
    // An uninitialised submodule is the environment's fault, not the spec's.
    if (!existsSync(join(DEFAULT_ROOT, "vendor", spec.submodule, "Cargo.toml")))
      continue;
    assert.ok(existsSync(manifest), `${spec.id}: ${spec.manifest} is missing`);
  }
});

test("an unknown module id is refused with the known ids named", () => {
  assert.throws(
    () => main(["no-such-module"]),
    /unknown module id "no-such-module" \(known: .*tinydocs/,
  );
  const cli = spawnSync(
    process.execPath,
    [fileURLToPath(new URL("../modules/build.mjs", import.meta.url)), "nope"],
    { encoding: "utf8" },
  );
  assert.notEqual(cli.status, 0);
  assert.match(cli.stderr, /unknown module id "nope"/);
});

test("options are validated", () => {
  assert.throws(() => main([]), /usage/);
  assert.throws(() => main(["all", "--profile", "fast"]), /--profile/);
  assert.throws(() => main(["all", "--bogus"]), /unknown option/);
});

function fakeRepo() {
  const root = mkdtempSync(join(tmpdir(), "module-build-"));
  const modules = join(root, "crates/openhuman-core/src/modules");
  mkdirSync(join(modules, "registry"), { recursive: true });
  writeFileSync(join(modules, "registry.rs"), "pub const ALL: &[ModuleRecord] = &[TINYX];\n");
  writeFileSync(
    join(modules, "registry/records_x.rs"),
    `pub(crate) const TINYX: ModuleRecord = ModuleRecord {
    id: "tinyx",
    load: LoadPolicy::Lazy,
    source: SourceSpec {
        submodule: "tinyx",
        manifest: "crates/tinyx-module/Cargo.toml",
        package: "tinyx-module",
        lib_name: "tinyx_module",
        features: &["extra"],
    },
};\n`,
  );
  const sub = join(root, "vendor/tinyx");
  mkdirSync(sub, { recursive: true });
  writeFileSync(join(sub, "Cargo.toml"), "# fixture\n");
  const git = (...args) =>
    spawnSync("git", ["-C", sub, "-c", "user.name=t", "-c", "user.email=t@t", ...args], {
      encoding: "utf8",
    });
  git("init", "-q");
  git("add", ".");
  git("commit", "-q", "-m", "fixture");
  const log = join(root, "cargo.log");
  const built = join(root, "built-module");
  const artifact = join(root, artifactFilename(SPEC));
  const cargo = join(root, "fake-cargo");
  writeFileSync(
    cargo,
    `#!/usr/bin/env bash
printf '%s\\n' "$*" >> "${log}"
printf 'native bytes' > "${built}"
printf '{"reason":"compiler-artifact","target":{"kind":["cdylib","rlib"]},"filenames":["${artifact}"]}\\n'
cp "${built}" "${artifact}"
`,
  );
  chmodSync(cargo, 0o755);
  return { root, log, cargo };
}

test("a build stages the artifact with its digest, and the second run is a cache hit", () => {
  const { root, log, cargo } = fakeRepo();
  try {
    const out = join(root, "out");
    const env = { ...process.env, CARGO: cargo, CI_CACHE_DIR: "", OPENHUMAN_MODULE_PATH: "" };
    const first = main(["tinyx", "--out", out], env, root);
    const name = artifactFilename(SPEC);
    assert.equal(readFileSync(join(out, "tinyx", name), "utf8"), "native bytes");
    assert.equal(
      readFileSync(join(out, "tinyx", "modules.toml"), "utf8"),
      `"${name}" = "${sha256Hex(Buffer.from("native bytes"))}"\n`,
    );
    assert.match(first, new RegExp(`^OPENHUMAN_MODULE_SOURCE_DIR=${out}$`, "m"));
    assert.match(first, /^OPENHUMAN_MODULE_SOURCE_TINYX=source$/m);
    assert.match(first, new RegExp(`^OPENHUMAN_MODULE_PATH=${join(out, "tinyx")}$`, "m"));
    const cargoRuns = () => readFileSync(log, "utf8").trim().split("\n");
    assert.equal(cargoRuns().length, 1);
    assert.match(cargoRuns()[0], /-p tinyx-module .*--release --features extra/);
    assert.match(cargoRuns()[0], /--manifest-path .*vendor\/tinyx\/crates\/tinyx-module\/Cargo\.toml/);
    assert.doesNotMatch(cargoRuns()[0], /--target-dir/);

    // Same inputs: served from the blob cache, cargo is not invoked again.
    const second = main(["tinyx", "--out", out, "--print-env"], env, root);
    assert.equal(cargoRuns().length, 1);
    assert.match(second, /^export OPENHUMAN_MODULE_SOURCE_TINYX='source'$/m);

    // A different profile is a different key and builds again.
    main(["tinyx", "--out", out, "--profile", "dev"], env, root);
    assert.equal(cargoRuns().length, 2);
    assert.doesNotMatch(cargoRuns()[1], /--release/);

    // An uncommitted change to the submodule invalidates the key.
    writeFileSync(join(root, "vendor/tinyx/Cargo.toml"), "# edited\n");
    main(["tinyx", "--out", out, "--profile", "dev"], env, root);
    assert.equal(cargoRuns().length, 3);
    assert.ok(readdirSync(join(out, ".blobs")).length >= 3);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
