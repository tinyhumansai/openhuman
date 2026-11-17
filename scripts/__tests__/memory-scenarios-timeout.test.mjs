import assert from "node:assert/strict";
import { test } from "node:test";

import { extendedPreTurnWait } from "../memory-scenarios/scenarios.mjs";

test("the comparison wait stays above the configured pre-turn deadline", () => {
  assert.equal(extendedPreTurnWait(10_000), 15_000);
  assert.equal(extendedPreTurnWait(20_000), 25_000);
  assert.throws(() => extendedPreTurnWait(undefined), /positive safe integer/);
});
