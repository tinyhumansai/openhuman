// Guards the Linux .deb runtime dependency list in tauri.conf.json (moved from
// the former tests/linux_cef_deb_runtime_e2e.rs). libxdo3 in particular keeps
// loaded Computer module usable even though the host no longer links it.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");

test("tauri.conf.json linux deb depends lists the required runtime libraries once", () => {
  const config = JSON.parse(
    readFileSync(resolve(root, "crates/openhuman-app/tauri.conf.json"), "utf8"),
  );
  const deps = config?.bundle?.linux?.deb?.depends;
  assert.ok(Array.isArray(deps), "bundle.linux.deb.depends should be an array");
  for (const dep of [
    "libgtk-3-0",
    "libwebkit2gtk-4.1-0",
    "libx11-6",
    "libxdo3",
    "libgdk-pixbuf-2.0-0",
    "libglib2.0-0",
  ]) {
    assert.ok(deps.includes(dep), `deb depends missing ${dep}`);
    assert.match(dep, /^[a-z0-9][a-z0-9.+-]*[a-z0-9]$/, `invalid Debian package name ${dep}`);
  }
  assert.equal(deps.filter((d) => d === "libxdo3").length, 1, "libxdo3 should be listed exactly once");
});

// Loaded audio modules need ALSA even after host-side audio dependencies go away.
test("Linux deb declares the ALSA runtime for both supported Ubuntu package names", () => {
  const config = JSON.parse(
    readFileSync(resolve(root, "crates/openhuman-app/tauri.conf.json"), "utf8"),
  );
  const deps = config.bundle.linux.deb.depends;
  assert.equal(deps.filter((dep) => dep === "libasound2t64 | libasound2").length, 1);
});
