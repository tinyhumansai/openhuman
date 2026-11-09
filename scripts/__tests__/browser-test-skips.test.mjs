import assert from "node:assert/strict";
import test from "node:test";

import { untrackedSkipLines } from "../ci/check-browser-test-skips.mjs";

const url = "https://github.com/tinyhumansai/openhuman/issues/123";

test("requires a same-line URL for direct, qualified, and runtime skip calls", () => {
  const source = [
    `it.skip('tracked', () => {}); // ${url}`,
    `test . skip /* reason */ ('tracked', () => {}); // ${url}`,
    `test.describe /* reason */ . skip ('tracked', () => {}); // ${url}`,
    `describe /* reason */ . skip ('tracked suite', () => {}); // ${url}`,
    `describe.\n  skip ('missing');`,
    `this /* runtime */ . skip /* reason */ ( );`,
    `test.step.skip('missing');`,
  ].join("\n");

  assert.deepEqual(untrackedSkipLines(source), [
    "6: skip ('missing');",
    "7: this /* runtime */ . skip /* reason */ ( );",
    "8: test.step.skip('missing');",
  ]);
});

test("ignores inert text but scans executable template interpolations", () => {
  const source = [
    "// test.skip('comment')",
    "/* test.skip('block comment') */",
    "const example = \"it.skip('string')\";",
    "const template = `describe.skip('template')`;",
    "const interpolated = `raw test.skip('ignored') ${test.skip('missing')}`;",
    `const linked = \`raw describe.skip('ignored') \${test.skip('tracked')}\`; // ${url}`,
    "const nested = `outer ${({ value: `inner ${this.skip()}` })}`;",
    "const removed = '- test.skip(\\\"old\\\")';",
    `test.skip('real'); // ${url}`,
  ].join("\n");

  assert.deepEqual(untrackedSkipLines(source), [
    "5: const interpolated = `raw test.skip('ignored') ${test.skip('missing')}`;",
    "7: const nested = `outer ${({ value: `inner ${this.skip()}` })}`;",
  ]);
});

test("ignores regex literals without letting their delimiters hide executable calls", () => {
  const source = [
    'const quote = /"/;',
    "test.skip('missing after quote regex');",
    "const backtick = /`/;",
    "this.skip();",
    "const fakeCall = /test.skip\\(foo\\)/;",
    "const commentStart = /\\/\\//;",
    "const blockCommentStart = /\\/\\*/;",
    `test.skip('tracked'); // ${url}`,
  ].join("\n");

  assert.deepEqual(untrackedSkipLines(source), [
    "2: test.skip('missing after quote regex');",
    "4: this.skip();",
  ]);
});

test("parses TypeScript and TSX using each spec's actual language mode", () => {
  const typescript = [
    "const asserted = <number>other;",
    "it.skip('unlinked after angle assertion');",
    "const identity = <T>(value: T) => value;",
    "test.skip('unlinked after generic arrow');",
  ].join("\n");
  const tsx = [
    "const element = <section />;",
    "describe.skip('unlinked after JSX');",
  ].join("\n");

  assert.deepEqual(untrackedSkipLines(typescript, "fixture.spec.ts"), [
    "2: it.skip('unlinked after angle assertion');",
    "4: test.skip('unlinked after generic arrow');",
  ]);
  assert.deepEqual(untrackedSkipLines(tsx, "fixture.spec.tsx"), [
    "2: describe.skip('unlinked after JSX');",
  ]);
});

test("fails closed when a browser spec cannot be parsed", () => {
  assert.throws(
    () =>
      untrackedSkipLines("const = ;\ntest.skip('unlinked');", "broken.spec.ts"),
    /Could not parse broken\.spec\.ts at line 1/,
  );
});
