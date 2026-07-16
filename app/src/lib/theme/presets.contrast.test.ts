import { readFileSync } from 'node:fs';
import { dirname, resolve as resolvePath } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

import { channelContrast } from './color';
import { PRESET_THEMES } from './presets';
import type { Theme } from './types';

/**
 * WCAG AA gate for every shipped **dark** preset.
 *
 * A preset only carries the tokens it overrides; everything else falls through
 * to the `:root.dark` defaults in `app/src/styles/tokens.css`. Those defaults are
 * not importable as JS, so we mirror the relevant subset here and resolve each
 * theme by merging its `colors` over this base — the same layering ThemeProvider
 * does at runtime.
 *
 * This mirror is not trusted blindly: the `DARK_BASE parity` test below parses
 * tokens.css and fails if any value here drifts from the CSS source of truth, so
 * a future token edit can't leave this gate green while runtime colours change.
 */
const DARK_BASE: Record<string, string> = {
  surface: '23 23 23',
  'surface-canvas': '0 0 0',
  'surface-muted': '38 38 38',
  'surface-subtle': '38 38 38',
  'surface-strong': '38 38 38',
  'surface-hover': '38 38 38',
  'surface-overlay': '0 0 0',
  content: '245 245 245',
  'content-secondary': '212 212 212',
  'content-muted': '163 163 163',
  'content-faint': '115 115 115',
  'content-inverted': '255 255 255',
  'primary-200': '191 219 254',
  'primary-300': '147 197 253',
  'primary-400': '96 165 250',
  'primary-500': '47 110 244',
  'primary-600': '37 99 235',
  'primary-700': '29 78 216',
};

const AA_TEXT = 4.5; // body text
const AA_LARGE = 3.0; // large text / UI elements / disabled-placeholder

/**
 * Every surface layer text can land on — base, canvas, recessed wells, and the
 * hover/pressed states — plus `surface-overlay` (the modal scrim, tested as a
 * solid fill, which is the conservative worst case since it renders at < full
 * opacity over another surface).
 */
const SURFACES = [
  'surface',
  'surface-canvas',
  'surface-muted',
  'surface-subtle',
  'surface-strong',
  'surface-hover',
  'surface-overlay',
] as const;

/** Text tiers held to full body contrast against every surface. */
const BODY_TIERS = ['content', 'content-secondary', 'content-muted'] as const;

function resolve(theme: Theme): Record<string, string> {
  return { ...DARK_BASE, ...theme.colors };
}

/**
 * Parse the `--token: R G B;` declarations inside a single CSS rule block
 * (`selector { … }`) from tokens.css into a `{ token: 'R G B' }` map. The theme
 * blocks contain no nested braces, so a non-greedy `{ … }` match is sufficient.
 */
function parseTokenBlock(css: string, selector: string): Record<string, string> {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const match = css.match(new RegExp(`${escaped}\\s*\\{([^}]*)\\}`));
  if (!match) throw new Error(`tokens.css: block not found for "${selector}"`);
  const out: Record<string, string> = {};
  for (const line of match[1].split('\n')) {
    const decl = line.match(/^\s*--([a-z0-9-]+):\s*([^;]+);/i);
    if (decl) out[decl[1]] = decl[2].trim();
  }
  return out;
}

// Effective dark values = `:root` defaults overlaid by `:root.dark` — the same
// cascade the browser applies. Accents (primary-*) live only in `:root` and are
// inherited under dark, exactly as DARK_BASE encodes them.
const tokensCss = readFileSync(
  resolvePath(dirname(fileURLToPath(import.meta.url)), '../../styles/tokens.css'),
  'utf8'
);
const effectiveDarkTokens = {
  ...parseTokenBlock(tokensCss, ':root'),
  ...parseTokenBlock(tokensCss, ':root.dark'),
};

describe('DARK_BASE parity with tokens.css', () => {
  // If this fails, a tokens.css edit drifted from the mirror above — update
  // DARK_BASE (and re-check the AA gate) rather than muting this test.
  for (const key of Object.keys(DARK_BASE)) {
    it(`--${key} matches the CSS source of truth`, () => {
      expect(effectiveDarkTokens[key], `token --${key}`).toBe(DARK_BASE[key]);
    });
  }
});

const darkPresets = PRESET_THEMES.filter(t => t.isDark && t.builtIn);

describe('preset dark themes meet WCAG AA', () => {
  it('ships the expected dark presets', () => {
    // Guards against a preset being renamed/dropped without updating this gate.
    expect(darkPresets.map(t => t.id).sort()).toEqual(
      ['dark', 'hal9000', 'matrix', 'ocean-dark', 'sepia-dark'].sort()
    );
  });

  for (const theme of darkPresets) {
    describe(`${theme.name} (${theme.id})`, () => {
      const t = resolve(theme);

      it('body/muted text ≥ 4.5:1 on every surface state', () => {
        for (const surface of SURFACES) {
          for (const tier of BODY_TIERS) {
            const ratio = channelContrast(t[tier], t[surface]);
            expect(
              ratio,
              `${theme.id}: ${tier} on ${surface} = ${ratio.toFixed(2)}`
            ).toBeGreaterThanOrEqual(AA_TEXT);
          }
        }
      });

      it('faint/placeholder text ≥ 3:1 on every surface state', () => {
        for (const surface of SURFACES) {
          const ratio = channelContrast(t['content-faint'], t[surface]);
          expect(
            ratio,
            `${theme.id}: content-faint on ${surface} = ${ratio.toFixed(2)}`
          ).toBeGreaterThanOrEqual(AA_LARGE);
        }
      });

      it('primary button label ≥ 4.5:1 on its resting and active fills', () => {
        // Button.tsx: bg-primary-500 (rest) / dark:active:bg-primary-600, label
        // is text-content-inverted. The transient dark-mode hover fill
        // (dark:hover:bg-primary-400) is deliberately NOT gated here: it lightens
        // the fill app-wide, so even the untouched historical `dark` preset sits
        // at ~2.5:1 white-on-primary-400. That is a shared Button behaviour, not a
        // per-theme token, and fixing it needs a Button change, not a palette one.
        for (const shade of ['primary-500', 'primary-600'] as const) {
          const ratio = channelContrast(t['content-inverted'], t[shade]);
          expect(
            ratio,
            `${theme.id}: content-inverted on ${shade} = ${ratio.toFixed(2)}`
          ).toBeGreaterThanOrEqual(AA_TEXT);
        }
      });

      it('accent/link text ≥ 4.5:1 on surface and canvas', () => {
        // Dark-mode accent text uses the lighter shades (dark:text-primary-200…400).
        for (const shade of ['primary-200', 'primary-300', 'primary-400'] as const) {
          for (const surface of ['surface', 'surface-canvas'] as const) {
            const ratio = channelContrast(t[shade], t[surface]);
            expect(
              ratio,
              `${theme.id}: ${shade} text on ${surface} = ${ratio.toFixed(2)}`
            ).toBeGreaterThanOrEqual(AA_TEXT);
          }
        }
      });

      it('primary-500 reads as a UI element ≥ 3:1 on every surface', () => {
        // Focus ring (focus-visible:ring-primary-500), button fills, and control
        // boundaries can sit on any surface layer, so hold the bar on all of them.
        for (const surface of SURFACES) {
          const ratio = channelContrast(t['primary-500'], t[surface]);
          expect(
            ratio,
            `${theme.id}: primary-500 vs ${surface} = ${ratio.toFixed(2)}`
          ).toBeGreaterThanOrEqual(AA_LARGE);
        }
      });
    });
  }
});
