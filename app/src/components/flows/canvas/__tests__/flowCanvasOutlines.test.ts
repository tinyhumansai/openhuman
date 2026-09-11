/**
 * Guard: the flow canvas draws every one of its status rings with `outline-solid`,
 * so a global `outline-solid` suppression silently kills all of them at once.
 *
 * `app/src/index.css` previously carried `outline-solid: none !important` on the bare
 * universal selector. Its comment says it exists to hide the browser focus
 * ring — but `!important` on `*` outranks every component rule, so it also
 * suppressed the live run overlay (`.flow-node-running` / `-success` /
 * `-failed`), the validation ring (`.flow-node-error`) and the copilot diff
 * overlay (`.flow-node-added` / `-removed`). All four rendered nothing, in
 * every build, with no test failing — the classes were applied correctly to the
 * DOM the whole time, so only a human looking at the canvas could notice.
 *
 * jsdom does not implement the cascade well enough to assert the computed
 * outline, so this asserts the source invariant instead: focus-ring suppression
 * must be scoped to a focus state, never applied unconditionally to `*`.
 */
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

// Vitest runs with `app/` as cwd (see test/vitest.config.ts).
const INDEX_CSS = resolve(process.cwd(), 'src/index.css');
const CANVAS_CSS = resolve(process.cwd(), 'src/components/flows/canvas/flowCanvasStyles.css');

/** Strips comments so prose mentioning a rule is never mistaken for the rule. */
function withoutComments(css: string): string {
  return css.replace(/\/\*[\s\S]*?\*\//g, '');
}

describe('flow canvas status rings', () => {
  it('are not suppressed by an unconditional universal outline-solid reset', () => {
    const css = withoutComments(readFileSync(INDEX_CSS, 'utf8'));
    // Find every `*` block that is NOT tied to a focus state.
    const universalBlocks = [...css.matchAll(/(^|\})\s*\*\s*\{([^}]*)\}/g)].map(m => m[2]);
    for (const block of universalBlocks) {
      expect(
        /outline\s*:/.test(block),
        'index.css must not set `outline-solid` on the bare `*` selector — with `!important` it ' +
          'outranks every flow-node ring-3 rule and silently blanks the canvas overlays. ' +
          'Scope focus-ring suppression to `*:focus` (and the explicit element list) instead.'
      ).toBe(false);
    }
  });

  it('still declare an outline-solid for each run/validation/diff state', () => {
    const css = withoutComments(readFileSync(CANVAS_CSS, 'utf8'));
    for (const cls of [
      'flow-node-running',
      'flow-node-success',
      'flow-node-failed',
      'flow-node-error',
      'flow-node-added',
      'flow-node-removed',
    ]) {
      const rule = new RegExp(`\\.${cls}\\s*\\{[^}]*outline\\s*:`);
      expect(rule.test(css), `${cls} must declare an outline-solid`).toBe(true);
    }
  });

  it('keeps the running state animated so an executing node is distinguishable', () => {
    const css = withoutComments(readFileSync(CANVAS_CSS, 'utf8'));
    expect(/\.flow-node-running\s*\{[^}]*animation\s*:/.test(css)).toBe(true);
    expect(css).toContain('@keyframes flow-node-run-pulse');
  });
});
