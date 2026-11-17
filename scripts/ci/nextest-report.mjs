#!/usr/bin/env node
// Consume nextest's JUnit vocabulary; retain counts before checking inventory.
// Usage: node scripts/ci/nextest-report.mjs <junit.xml> <counts.json>
//        [newline-delimited-targets.txt] [nextest-list.json]
// NEXTEST_ELAPSED_SECONDS supplies the recorded command duration. Exit 1 means
// missing/empty reporting or an unmet inventory expectation; test failures are
// recorded here, while the calling wrapper preserves the test runner's status.
import fs from "node:fs";

const [report, output, expectedFile, inventoryFile] = process.argv.slice(2);
if (!fs.existsSync(report)) {
  console.error(`[ci][nextest] JUnit report unavailable: ${report}`);
  process.exit(1);
}
const xml = fs.readFileSync(report, "utf8");
const decode = (value) =>
  value
    .replace(/&quot;/g, '"')
    .replace(/&apos;/g, "'")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&amp;/g, "&");
const suites = [...xml.matchAll(/<testsuite\s+([^>]+)>/g)].map((match) => {
  const attributes = Object.fromEntries(
    [...match[1].matchAll(/([\w-]+)="([^"]*)"/g)].map((entry) => [
      entry[1],
      decode(entry[2]),
    ]),
  );
  return {
    name: attributes.name,
    tests: Number(attributes.tests ?? 0),
    failures: Number(attributes.failures ?? 0),
    errors: Number(attributes.errors ?? 0),
    skipped: Number(attributes.skipped ?? 0),
  };
});
const counts = {
  elapsedSeconds: Number(process.env.NEXTEST_ELAPSED_SECONDS ?? 0),
  tests: 0,
  failures: 0,
  errors: 0,
  skipped: 0,
  suites,
};
for (const suite of suites)
  for (const key of ["tests", "failures", "errors", "skipped"])
    counts[key] += suite[key];
fs.writeFileSync(output, `${JSON.stringify(counts, null, 2)}\n`);
console.log(
  `[ci][nextest] ${report}: ${counts.tests} tests, ${counts.failures} failures, ${counts.errors} errors, ${counts.skipped} skipped`,
);
const expected = expectedFile
  ? fs.readFileSync(expectedFile, "utf8").split(/\r?\n/).filter(Boolean)
  : [];
const inventory = inventoryFile
  ? Object.values(
      JSON.parse(fs.readFileSync(inventoryFile, "utf8"))["rust-suites"] ?? {},
    )
  : null;
const missing = expected.filter((target) => {
  if (inventory) {
    const binary = inventory.find((suite) => suite["binary-name"] === target);
    // A missing compiled target still fails. Only a present binary whose
    // complete selected inventory is ignored may omit its JUnit testsuite.
    if (!binary) return true;
    const selected = Object.values(binary.testcases ?? {});
    if (selected.length > 0 && selected.every((test) => test.ignored === true))
      return false;
  }
  return !suites.some(
    (suite) =>
      (suite.name === target || suite.name?.endsWith(`::${target}`)) &&
      suite.tests > 0,
  );
});
if (counts.tests === 0 || missing.length > 0) {
  console.error(
    `[ci][nextest] expected nonempty suites; missing/empty: ${missing.join(", ") || "entire selection"}`,
  );
  process.exitCode = 1;
}
