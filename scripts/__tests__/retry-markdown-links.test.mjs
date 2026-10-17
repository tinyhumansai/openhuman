import assert from "node:assert/strict";
import test from "node:test";
import {
  retryServiceUnavailable,
  serviceUnavailableUrls,
} from "../ci/retry-markdown-links.mjs";

const unavailable =
  "* [503] <https://example.test/docs#anchor> | Service Unavailable\n";
const internalError =
  "* [500] <https://example.test/docs#anchor> | Internal Server Error\n";
const missing = "* [404] <https://example.test/missing> | Not Found\n";

function checker(results) {
  const calls = [];
  return {
    calls,
    check: async (urls) => {
      calls.push(urls);
      return results[calls.length - 1];
    },
  };
}

test("transient 503 recovers after more than one retry and preserves fragments", async () => {
  const { calls, check } = checker([
    { status: 2, report: unavailable },
    { status: 0, report: "" },
  ]);
  const waits = [];
  const logs = [];
  assert.equal(
    await retryServiceUnavailable({
      initialReport: unavailable,
      check,
      wait: async (ms) => waits.push(ms),
      log: (message) => logs.push(message),
    }),
    0,
  );
  assert.deepEqual(calls, [
    ["https://example.test/docs#anchor"],
    ["https://example.test/docs#anchor"],
  ]);
  assert.deepEqual(waits, [5000, 10000]);
  assert.ok(logs.some((line) => /attempt=1 backoff_ms=5000 urls=1/.test(line)));
  assert.ok(logs.some((line) => /attempt=1 status=2/.test(line)));
  assert.ok(logs.some((line) => /attempt=2 status=0/.test(line)));
  assert.ok(logs.some((line) => /reason=recovered/.test(line)));
});

test("transient GitHub 500 responses are retried too", async () => {
  const { calls, check } = checker([
    { status: 2, report: internalError },
    { status: 0, report: "" },
  ]);
  assert.equal(
    await retryServiceUnavailable({
      initialReport: internalError,
      check,
      wait: async () => {},
    }),
    0,
  );
  assert.deepEqual(calls, [
    ["https://example.test/docs#anchor"],
    ["https://example.test/docs#anchor"],
  ]);
});

test("persistent 503 remains a failure after bounded retries", async () => {
  const { calls, check } = checker(
    Array.from({ length: 3 }, () => ({ status: 2, report: unavailable })),
  );
  assert.equal(
    await retryServiceUnavailable({
      initialReport: unavailable,
      check,
      wait: async () => {},
    }),
    2,
  );
  assert.equal(calls.length, 3);
});

test("404 or mixed failures are not retried", async () => {
  for (const initialReport of [
    missing,
    unavailable + missing,
    "",
    "* [TIMEOUT] <https://example.test/> | Timeout\n",
  ]) {
    const { calls, check } = checker([]);
    assert.equal(
      await retryServiceUnavailable({
        initialReport,
        check,
        wait: async () => {},
      }),
      1,
    );
    assert.equal(calls.length, 0);
  }
});

test("a permanent error discovered on retry remains a failure", async () => {
  const { calls, check } = checker([{ status: 2, report: missing }]);
  assert.equal(
    await retryServiceUnavailable({
      initialReport: unavailable,
      check,
      wait: async () => {},
    }),
    2,
  );
  assert.equal(calls.length, 1);
});

test("retry inputs deduplicate repeated failures without dropping URL fragments", () => {
  assert.deepEqual(serviceUnavailableUrls(unavailable.repeat(3)), [
    "https://example.test/docs#anchor",
  ]);
});

test("executable retries preserve report files and propagate final success or failure", async () => {
  const { mkdtempSync, writeFileSync, readFileSync, rmSync } =
    await import("node:fs");
  const { tmpdir } = await import("node:os");
  const { join } = await import("node:path");
  const { recheckReport } = await import("../ci/retry-markdown-links.mjs");
  for (const recover of [true, false]) {
    const folder = mkdtempSync(join(tmpdir(), "lychee retry test "));
    try {
      const report = join(folder, "report.md");
      const binary = join(folder, "fake checker.mjs");
      const counter = join(folder, "calls.json");
      writeFileSync(report, unavailable);
      writeFileSync(
        binary,
        `#!/usr/bin/env node
import { readFileSync, writeFileSync, existsSync } from 'node:fs';
const counter = ${JSON.stringify(counter)};
const count = existsSync(counter) ? JSON.parse(readFileSync(counter, 'utf8')) + 1 : 1;
writeFileSync(counter, JSON.stringify(count));
const args = process.argv.slice(2);
const input = readFileSync(args.at(-1), 'utf8');
if (!input.includes('https://example.test/docs#anchor')) process.exit(3);
const success = ${recover} && count === 2;
writeFileSync(args[args.indexOf('--output') + 1], success ? '' : ${JSON.stringify(unavailable)});
process.exit(success ? 0 : 2);
`,
        { mode: 0o755 },
      );
      assert.equal(
        await recheckReport(report, binary, { wait: async () => {} }),
        recover ? 0 : 2,
      );
      assert.equal(JSON.parse(readFileSync(counter, "utf8")), recover ? 2 : 3);
      assert.equal(
        readFileSync(join(folder, "retry-1.md"), "utf8"),
        "- <https://example.test/docs#anchor>\n",
      );
    } finally {
      rmSync(folder, { recursive: true, force: true });
    }
  }
});
