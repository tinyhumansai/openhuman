/**
 * Regression: a centered dialog opened offset from center and snapped into
 * place when its entrance animation finished.
 *
 * `DialogContent` / `AlertDialogContent` center with `fixed left-1/2 top-1/2`
 * plus `-translate-x-1/2 -translate-y-1/2`. Under Tailwind v4 those utilities
 * compile to the **`translate` property**, not to `transform`:
 *
 *     .-translate-x-1\/2 { --tw-translate-x: …; translate: var(--tw-translate-x) var(--tw-translate-y) }
 *
 * `translate` and `transform` compose rather than override, so a keyframe that
 * sets `transform: translate(-50%, …)` — correct under v3, where the utilities
 * WERE transforms — *adds* to the utility and opens the dialog at -100%/-100%.
 *
 * Asserting the class name alone would only catch a literal revert. These tests
 * assert the RULE instead, against the real Tailwind v4 theme: the entrance
 * animation must drive the same property the centering utilities use, and must
 * land exactly where they leave it so nothing moves when the animation hands
 * the element back.
 */
import { render, screen } from '@testing-library/react';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

import AlertDialogRoot, { AlertDialogContent } from './AlertDialog';
import { DialogContent, DialogRoot } from './Dialog';

/** The animation registrations and keyframes, read from the real theme. */
const tailwindSource = readFileSync(join(__dirname, '..', '..', 'index.css'), 'utf8');

/** `--animate-dialog-in: dialogIn 0.2s …` → the keyframe name `dialogIn`. */
const keyframeNameFor = (animation: string): string => {
  const match = tailwindSource.match(new RegExp(`--animate-${animation}:\\s*(\\w+)`));
  if (!match) throw new Error(`no animation registered as '${animation}'`);
  return match[1];
};

/** Every value of `property` declared inside the named keyframe block. */
const declaredIn = (keyframe: string, property: string): string[] => {
  const start = tailwindSource.indexOf(`  @keyframes ${keyframe} {`);
  if (start === -1) throw new Error(`no keyframes named ${keyframe}`);
  const end = tailwindSource.indexOf('\n  }', start);
  const block = tailwindSource.slice(start, end);
  return Array.from(block.matchAll(new RegExp(`(?:^|[;{\\s])${property}:\\s*([^;]+);`, 'g'))).map(
    m => m[1].trim()
  );
};

const centeringClassesOf = (el: HTMLElement) => ({
  centersX: el.className.includes('-translate-x-1/2'),
  centersY: el.className.includes('-translate-y-1/2'),
  animation: /animate-([\w-]+)/.exec(el.className)?.[1],
});

describe('centered dialog entrance animation', () => {
  it.each([
    [
      'DialogContent',
      () => (
        <DialogRoot open>
          <DialogContent aria-label="d">body</DialogContent>
        </DialogRoot>
      ),
    ],
    [
      'AlertDialogContent',
      () => (
        <AlertDialogRoot open>
          <AlertDialogContent aria-label="d">body</AlertDialogContent>
        </AlertDialogRoot>
      ),
    ],
  ])('%s never has its centering displaced by the animation', (_name, renderIt) => {
    render(renderIt());
    const content = screen.getByText('body').closest('[data-slot$="dialog-content"]');
    expect(content).not.toBeNull();

    const { centersX, centersY, animation } = centeringClassesOf(content as HTMLElement);
    // Guard the premise: if it ever stops centering by translate utilities,
    // this whole rule is moot and the test should be revisited rather than
    // silently pass.
    expect(centersX && centersY).toBe(true);
    expect(animation).toBeDefined();

    const keyframe = keyframeNameFor(animation!);

    // A `transform` frame composes ON TOP of the utilities' `translate`, so it
    // must not carry a translation of its own. (Scale, rotate and opacity are
    // all fine — they do not displace the element's center.)
    for (const transform of declaredIn(keyframe, 'transform')) {
      expect(transform, `keyframe of ${animation} translates via transform`).not.toMatch(
        /translate/
      );
    }

    // Whatever it does animate must land on the utilities' resting value, or
    // the dialog jumps as the animation releases the element.
    const translates = declaredIn(keyframe, 'translate');
    expect(translates.length, `keyframe of ${animation} animates no translate`).toBeGreaterThan(0);
    expect(translates.at(-1)).toBe('-50% -50%');
  });

  it('starts near its resting place rather than at the viewport origin', () => {
    // The opening frame must already carry the centering. Without it the
    // element starts at the top-left corner of the viewport and flies in.
    const [first] = declaredIn(keyframeNameFor('dialog-in'), 'translate');
    expect(first).toMatch(/^-50%/);
  });
});
