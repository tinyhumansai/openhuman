import type { Theme } from './types';

/**
 * Fill in the two "chrome" tokens for a theme that tints the app but does not
 * name them.
 *
 * The window chrome is the outermost surface the user sees — the tinted frame
 * the sidebar sits on, behind the inset content card — and `--line-chrome` is
 * the hairline where the card meets it. No preset set either, so picking a
 * themed palette recoloured every card, panel, border and text ramp and left
 * the frame around them the default neutral grey. The card went red; the app
 * stayed monotone.
 *
 * Deriving rather than hardcoding eight presets is deliberate: the same gap hits
 * any theme a user builds in the Theme Studio by tinting the canvas, and a
 * hardcoded table would not cover those. An explicit value always wins, so a
 * theme (or a Studio edit) that names a chrome token is never overridden.
 *
 * The ratios come from the defaults in `styles/tokens.css`, so an untinted theme
 * derives back to what it already had:
 *
 * - **Light** — canvas 245, chrome 214: chrome is ~13% darker. That direction is
 *   load-bearing, not taste. `RootShellLayout` paints the chrome at `/30` as a
 *   legibility scrim over the themed backdrop, and tokens.css records that the
 *   value was once stone-100 — identical to the canvas beneath it — which left
 *   the scrim doing nothing and a white card floating on white.
 * - **Dark** — canvas 0, chrome 10: chrome sits just *above* the canvas and
 *   below the surface (23), so the frame reads as behind the card rather than
 *   in front of it. An offset, not a ratio: scaling near-black by a factor moves
 *   nothing.
 *
 * `line-chrome` needs no arithmetic at all — in both default palettes it is
 * exactly `line-strong` (212 and 64), so it simply follows the theme's own
 * strong border.
 */

/** `"R G B"` → clamped channel triple, mapped component-wise. */
function mapChannels(channels: string, fn: (channel: number) => number): string | undefined {
  const parts = channels.trim().split(/\s+/);
  if (parts.length !== 3) return undefined;
  const mapped = parts.map(part => {
    const n = Number(part);
    if (!Number.isFinite(n)) return NaN;
    return Math.max(0, Math.min(255, Math.round(fn(n))));
  });
  return mapped.some(Number.isNaN) ? undefined : mapped.join(' ');
}

/** How much darker the light chrome sits than the light canvas (214 / 245). */
const LIGHT_CHROME_RATIO = 214 / 245;

/** How far the dark chrome sits above the dark canvas (10 − 0). */
const DARK_CHROME_OFFSET = 10;

/** Derive `surface-chrome` from a theme's canvas, respecting light vs dark. */
export function deriveSurfaceChrome(canvas: string, isDark: boolean): string | undefined {
  return isDark
    ? mapChannels(canvas, c => c + DARK_CHROME_OFFSET)
    : mapChannels(canvas, c => c * LIGHT_CHROME_RATIO);
}

/**
 * Return `theme` with `surface-chrome` / `line-chrome` filled in where the theme
 * tints the app but leaves them unset. Returns the SAME object when there is
 * nothing to add, so callers can keep using it as a render dependency.
 */
export function withDerivedChrome(theme: Theme): Theme {
  const derived: Record<string, string> = {};

  const canvas = theme.colors['surface-canvas'];
  if (canvas && !theme.colors['surface-chrome']) {
    const chrome = deriveSurfaceChrome(canvas, theme.isDark);
    if (chrome) derived['surface-chrome'] = chrome;
  }

  const lineStrong = theme.colors['line-strong'];
  if (lineStrong && !theme.colors['line-chrome']) {
    derived['line-chrome'] = lineStrong;
  }

  if (Object.keys(derived).length === 0) return theme;
  return { ...theme, colors: { ...theme.colors, ...derived } };
}
