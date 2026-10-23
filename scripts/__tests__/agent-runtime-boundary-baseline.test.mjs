import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const root = new URL("../../", import.meta.url);
const baselineUrl = new URL(
  "scripts/ci/agent-runtime-boundary-baseline.json",
  root,
);
const sourceUrl = new URL(
  "crates/openhuman-core/src/inbound/pinned.rs",
  root,
);

const identity = ({ rule, path, text, occurrence }) =>
  `${rule}\0${path}\0${text}\0${occurrence}`;

test("inbound pinned agents enter through the context-owned turn origin", async () => {
  const baseline = JSON.parse(await readFile(baselineUrl, "utf8"));
  const source = await readFile(sourceUrl, "utf8");
  const finding = {
    rule: "openhuman-task-local",
    path: "crates/openhuman-core/src/inbound/pinned.rs",
    line: 97,
    text: "let outcome = ctx",
    occurrence: 1,
  };

  assert.equal(source.split("\n")[96].trim(), finding.text);
  assert.equal(
    baseline.filter((entry) => identity(entry) === identity(finding)).length,
    0,
  );

  for (const unreviewed of [
    { ...finding, path: "crates/openhuman-core/src/inbound/other.rs" },
    { ...finding, text: "let outcome = crate::security::turn_origin::with_origin(" },
    { ...finding, text: "crate::security::turn_origin::capture()" },
  ]) {
    assert.equal(
      baseline.some((entry) => identity(entry) === identity(unreviewed)),
      false,
    );
  }
});
