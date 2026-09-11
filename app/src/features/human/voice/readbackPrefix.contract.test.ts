/**
 * The read-back prefix is a behavioural contract that spans two languages: the
 * renderer wraps a deferred answer in it, and the Rust harness recognises it to
 * answer the turn from the prompt instead of rebuilding an orchestrator — and to
 * stop speak-back re-arming into an unbounded loop.
 *
 * Both sides carry the string verbatim with a "MUST match" comment, but a comment
 * is not a check: if one side is edited, `readback_payload` silently stops
 * matching and the loop guard fails open, with every unit test on each side still
 * passing (they assert against their own copy). This reads the Rust source and
 * pins the two together, so a divergence fails here instead of in a live call.
 */
import { existsSync, readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

import { READBACK_PREFIX } from './useRealtimeVoiceSession';

const HARNESS_RELATIVE = 'src/openhuman/voice/realtime_harness_part_01.rs';

/**
 * Walk up from the working directory to the repo root. `import.meta.url` is not a
 * `file:` URL under vitest's transform, and the working directory differs between
 * running from `app/` and from the repo root — so anchor on the file itself.
 */
function findHarness(): string {
  let dir = process.cwd();
  for (;;) {
    const candidate = resolve(dir, HARNESS_RELATIVE);
    if (existsSync(candidate)) return candidate;
    const parent = dirname(dir);
    if (parent === dir)
      throw new Error(`could not locate ${HARNESS_RELATIVE} above ${process.cwd()}`);
    dir = parent;
  }
}

const HARNESS_PATH = findHarness();

describe('read-back prefix contract (TS ↔ Rust)', () => {
  const harness = readFileSync(HARNESS_PATH, 'utf8');

  it('finds the Rust constant where the contract says it lives', () => {
    // Guards the test itself: a moved or renamed constant would otherwise make
    // the assertion below vacuous rather than failing.
    expect(harness).toContain('const VOICE_READBACK_PREFIX: &str =');
  });

  it('matches the Rust VOICE_READBACK_PREFIX verbatim', () => {
    const match = harness.match(/const VOICE_READBACK_PREFIX: &str =\s*"([^"]*)"/);
    expect(match?.[1]).toBe(READBACK_PREFIX);
  });
});
