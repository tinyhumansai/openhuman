import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { dirname, posix, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const dockerfile = readFileSync(resolve(repoRoot, "Dockerfile"), "utf8");

/** COPY sources (repo-relative) of the named build stage. */
function stageCopies(name) {
  const lines = dockerfile.split("\n");
  const start = lines.findIndex((l) => new RegExp(`^FROM\\s.*\\sAS\\s+${name}\\s*$`, "i").test(l));
  assert.notEqual(start, -1, `Dockerfile has no "${name}" stage`);
  const copies = [];
  for (const line of lines.slice(start + 1)) {
    if (/^FROM\s/i.test(line)) break;
    const m = /^COPY\s+(?!--)(.+?)\s+\S+\s*$/.exec(line);
    if (m) copies.push(...m[1].split(/\s+/));
  }
  return copies;
}

function isCopied(path, copies) {
  return copies.some((c) => (c.endsWith("/") ? path.startsWith(c) : path === c));
}

test("Dockerfile modules stage copies every relative import of the .mjs files it copies", () => {
  const copies = stageCopies("modules");
  const queue = copies.filter((c) => c.endsWith(".mjs"));
  assert.ok(queue.length > 0, "modules stage copies no .mjs files");
  const seen = new Set();
  const missing = [];
  while (queue.length) {
    const file = queue.pop();
    if (seen.has(file)) continue;
    seen.add(file);
    const src = readFileSync(resolve(repoRoot, file), "utf8");
    const specs = [
      ...src.matchAll(/(?:^|\n)\s*(?:import|export)\b[^;'"]*?\bfrom\s*["'](\.{1,2}\/[^"']+)["']/g),
      ...src.matchAll(/(?:^|\n)\s*import\s*["'](\.{1,2}\/[^"']+)["']/g),
      ...src.matchAll(/\bimport\(\s*["'](\.{1,2}\/[^"']+)["']\s*\)/g),
    ].map((m) => m[1]);
    for (const spec of specs) {
      const target = posix.join(posix.dirname(file), spec);
      assert.ok(existsSync(resolve(repoRoot, target)), `${file} imports missing ${target}`);
      if (!isCopied(target, copies)) missing.push(`${file} imports ${target}`);
      else if (target.endsWith(".mjs")) queue.push(target);
    }
  }
  assert.deepEqual(missing, [], `modules stage does not COPY: ${missing.join("; ")}`);
});
