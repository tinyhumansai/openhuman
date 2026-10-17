// Regression for #6662: the updater manifest must carry a `windows-x86_64-msi`
// entry. tauri-plugin-updater resolves `<os>-<arch>-<installer>` for the
// bundle type the running copy came from, then falls back to `<os>-<arch>`
// (the NSIS setup). Without the `-msi` key an MSI (per-machine) install is
// "updated" by a per-user NSIS setup, leaving the old Program Files copy in
// place, so the app restarts back on the old version.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  chmodSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const script = resolve(
  dirname(fileURLToPath(import.meta.url)),
  "../release/publish-updater-manifest.sh",
);
const jqAvailable = spawnSync("jq", ["--version"], { encoding: "utf8" }).status === 0;

const ASSETS = [
  "OpenHuman_0.64.1_aarch64.app.tar.gz",
  "OpenHuman_0.64.1_x64.app.tar.gz",
  "OpenHuman_0.64.1_amd64.AppImage",
  "OpenHuman_0.64.1_aarch64.AppImage",
  "OpenHuman_0.64.1_x64-setup.exe",
  "OpenHuman_0.64.1_x64_en-US.msi",
];

function run(assets) {
  const dir = mkdtempSync(join(tmpdir(), "updater-manifest-"));
  try {
    const sigs = assets.map((a) => `${a}.sig`);
    writeFileSync(
      join(dir, "assets.txt"),
      [...assets, ...sigs].join("\n") + "\n",
    );
    // Fake `gh`: lists assets, writes a .sig on download, captures the upload.
    writeFileSync(
      join(dir, "gh"),
      `#!/usr/bin/env bash
case "$1 $2" in
  "release view") cat "${dir}/assets.txt" ;;
  "release download")
    while [ $# -gt 0 ]; do [ "$1" = "--pattern" ] && pat="$2"; [ "$1" = "--dir" ] && out="$2"; shift; done
    printf 'sig-for-%s\\n' "$pat" > "$out/$pat" ;;
  "release upload") cp "$4" "${dir}/uploaded.json" ;;
esac
`,
    );
    chmodSync(join(dir, "gh"), 0o755);
    const res = spawnSync("bash", [script], {
      env: {
        ...process.env,
        PATH: `${dir}:${process.env.PATH}`,
        TAG: "v0.64.1",
        VERSION: "0.64.1",
        REPO: "tinyhumansai/openhuman",
        GITHUB_TOKEN: "x",
      },
      encoding: "utf8",
    });
    let manifest = null;
    try {
      manifest = JSON.parse(readFileSync(join(dir, "uploaded.json"), "utf8"));
    } catch {}
    return { res, manifest };
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

test("manifest advertises both NSIS and MSI updater bundles for windows-x86_64", { skip: !jqAvailable }, () => {
  const { res, manifest } = run(ASSETS);
  assert.equal(res.status, 0, res.stderr);
  assert.match(manifest.platforms["windows-x86_64"].url, /x64-setup\.exe$/);
  assert.match(manifest.platforms["windows-x86_64-msi"].url, /x64_en-US\.msi$/);
  assert.match(
    manifest.platforms["windows-x86_64-msi"].signature,
    /x64_en-US\.msi\.sig/,
  );
});

test("publishing is refused when the MSI updater bundle is missing", { skip: !jqAvailable }, () => {
  const { res, manifest } = run(ASSETS.filter((a) => !a.endsWith(".msi")));
  assert.notEqual(res.status, 0);
  assert.match(res.stderr, /windows-x86_64-msi/);
  assert.equal(manifest, null);
});
