import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

const root = path.resolve(import.meta.dirname, "../..");
function fetchTags(inventorySource) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "module-tags-"));
  try {
    for (const folder of ["scripts/ci", "scripts/lib", "bin"])
      fs.mkdirSync(path.join(dir, folder), { recursive: true });
    fs.copyFileSync(
      path.join(root, "scripts/ci/fetch-submodule-tags.sh"),
      path.join(dir, "scripts/ci/fetch-submodule-tags.sh"),
    );
    if (inventorySource !== undefined)
      fs.writeFileSync(
        path.join(dir, "scripts/lib/module-pin-inventory.mjs"),
        inventorySource,
      );
    fs.writeFileSync(
      path.join(dir, "bin/git"),
      `#!${process.execPath}
const fs = require('node:fs');
const args = process.argv.slice(2);
fs.appendFileSync(process.env.FETCH_LOG, JSON.stringify(args) + '\\n');
if (args[2] === 'rev-parse') console.log(args[1] === 'vendor/tinycomputer' ? 'true' : 'false');
`,
      { mode: 0o755 },
    );
    execFileSync("bash", ["scripts/ci/fetch-submodule-tags.sh"], {
      cwd: dir,
      env: {
        ...process.env,
        PATH: `${dir}/bin:${process.env.PATH}`,
        FETCH_LOG: `${dir}/calls`,
      },
      stdio: "pipe",
    });
    return fs
      .readFileSync(`${dir}/calls`, "utf8")
      .trim()
      .split("\n")
      .map(JSON.parse);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
}

test("module tag fetch includes tinycomputer and unshallows only shallow modules", () => {
  const inventoryPath = path.join(root, "scripts/lib/module-pin-inventory.mjs");
  const calls = fetchTags(
    fs.existsSync(inventoryPath)
      ? fs.readFileSync(inventoryPath, "utf8")
      : undefined,
  );
  const fetches = calls.filter((args) => args[2] === "fetch");
  const computer = fetches.find((args) => args[1] === "vendor/tinycomputer");
  assert.ok(
    computer,
    "every module-pin source, including tinycomputer, needs its tags",
  );
  assert.ok(computer.includes("--unshallow"));
  assert.ok(
    fetches
      .find((args) => args[1] === "vendor/tinywallet")
      .every((arg) => arg !== "--unshallow"),
  );
  assert.ok(
    fetches.every(
      (args) =>
        args.includes("--tags") && args.includes("--no-recurse-submodules"),
    ),
  );
});

test("module inventory import is inert and new/shared/provider sources are fetched once", async () => {
  const { PIN_MAP, modulePinRepositories } =
    await import("../lib/module-pin-inventory.mjs");
  assert.ok(PIN_MAP.tinycomputer);
  const fixture = {
    original: { submodule: "vendor/original" },
    shared: { sharesWith: "vendor/original" },
    provider: {
      submodule: null,
      provider: { builtAgainst: "vendor/original" },
    },
    future: { submodule: "vendor/future" },
  };
  assert.deepEqual(modulePinRepositories(fixture), [
    "vendor/future",
    "vendor/original",
  ]);
  const source =
    fs.readFileSync(
      path.join(root, "scripts/lib/module-pin-inventory.mjs"),
      "utf8",
    ) +
    '\nPIN_MAP.future = { submodule: "vendor/future" };\nPIN_MAP.shared = { sharesWith: "vendor/tinycomputer" };\n';
  const fetches = fetchTags(source).filter((args) => args[2] === "fetch");
  assert.equal(
    fetches.filter((args) => args[1] === "vendor/tinycomputer").length,
    1,
  );
  assert.equal(fetches.filter((args) => args[1] === "vendor/future").length, 1);
});
