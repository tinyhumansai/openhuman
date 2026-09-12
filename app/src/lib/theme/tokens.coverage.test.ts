import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

import { ACCENT_FAMILIES, ACCENT_SHADES, COLOR_GROUPS } from './tokens';

/**
 * Drift guard: every colour token defined in `styles/tokens.css` must be
 * reachable from the Theme Studio.
 *
 * This exists because two of them were not, and nothing noticed. `surface-chrome`
 * (the window chrome behind the content card — the outermost background there
 * is) and `line-chrome` (the hairline where the card meets it) were both real,
 * themeable tokens that the editor simply never listed, so a custom theme could
 * restyle every card and panel and leave the frame around them untouched.
 *
 * A token is "reachable" if it is named in a {@link COLOR_GROUPS} group or is an
 * accent shade — those are surfaced per-family by the advanced expander rather
 * than listed individually, so the test reconstructs them the same way the panel
 * does instead of demanding they appear in a group.
 *
 * Parsing the stylesheet rather than a TypeScript mirror is the point: a mirror
 * is the thing that drifts. `tokens.css` is the single source of truth (its own
 * header says so), so adding a token there and forgetting the registry is what
 * this must catch.
 */
// Resolved from the Vitest root (`app/`), not from `import.meta.url`: the
// stylesheet is an asset rather than a module, so it has no module URL to hang
// a relative path off in this transform.
const TOKENS_CSS = resolve(process.cwd(), 'src/styles/tokens.css');

/** `--foo-bar: 12 34 56;` → `foo-bar`, scanning the light `:root` block only. */
function lightRootColorTokens(): string[] {
  const css = readFileSync(TOKENS_CSS, 'utf8');
  const start = css.indexOf(':root {');
  const darkStart = css.indexOf('.dark', start);
  const block = css.slice(start, darkStart > 0 ? darkStart : undefined);
  return [...block.matchAll(/--([a-z0-9-]+):\s*\d{1,3} \d{1,3} \d{1,3}/g)].map(m => m[1]);
}

describe('Theme Studio colour coverage', () => {
  const accentKeys = new Set(
    ACCENT_FAMILIES.flatMap(fam => ACCENT_SHADES.map(shade => `${fam}-${shade}`))
  );
  const groupKeys = new Set(COLOR_GROUPS.flatMap(g => g.keys));

  it('reads a non-trivial set of tokens out of tokens.css', () => {
    // Guards the guard: a regex that silently stops matching would make every
    // assertion below vacuously true.
    expect(lightRootColorTokens().length).toBeGreaterThan(40);
  });

  it('exposes every colour token defined in tokens.css', () => {
    const unreachable = lightRootColorTokens().filter(
      token => !groupKeys.has(token) && !accentKeys.has(token)
    );
    expect(unreachable).toEqual([]);
  });

  it('lists no token that tokens.css does not define', () => {
    const defined = new Set(lightRootColorTokens());
    const dangling = [...groupKeys].filter(key => !defined.has(key));
    expect(dangling).toEqual([]);
  });
});
