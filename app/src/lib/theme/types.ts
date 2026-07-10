import type { FontRole } from './tokens';

/**
 * A theme is a (partial) set of overrides for the canonical tokens in
 * `styles/tokens.css`. Anything omitted falls through to the tokens.css
 * Light/Dark defaults — so the built-in "Light" and "Dark" presets carry empty
 * override maps and simply lean on `isDark`.
 */
export interface Theme {
  /** Stable id (preset id or generated id for custom themes). */
  id: string;
  /** Display name. */
  name: string;
  /** Whether to apply the `.dark` class (selects the tokens.css dark base and
   *  keeps any remaining `dark:` utilities aligned). */
  isDark: boolean;
  /** Built-in presets cannot be edited in place — editing duplicates them. */
  builtIn: boolean;
  /**
   * For custom themes forked from a preset: the source preset variant id (e.g.
   * `ocean`, `matrix`). Lets "Reset overrides" restore the preset's base palette
   * rather than the generic Light/Dark defaults.
   */
  basedOn?: string;
  /** Colour token overrides, keyed by var name (no `--`) → `"R G B"` channels. */
  colors: Record<string, string>;
  /** Font role overrides → CSS font-family stack. */
  fonts: Partial<Record<FontRole, string>>;
  /**
   * Optional decorative gradients. `canvas` is a full CSS `background` value
   * applied to the app background (behind all surfaces); omit for a flat
   * canvas. Applied via the `--app-gradient` variable by ThemeProvider.
   */
  gradient?: { canvas?: string };
  /**
   * App backdrop layer. `mesh` (default) renders the animated WebGL mesh
   * gradient; `solid` shows just the flat/gradient canvas; `image` paints
   * `imageUrl` (cover). Controlled in the Theme Studio.
   */
  backdrop?: {
    kind: 'mesh' | 'solid' | 'image';
    imageUrl?: string;
    /** Show the dotted-canvas overlay (default true when omitted). */
    dots?: boolean;
  };
}

export type BackdropKind = 'mesh' | 'solid' | 'image';

/**
 * A theme family groups light and dark variants under one name so the picker can
 * offer "Ocean" with a Light/Dark/Auto choice rather than two separate entries.
 * At least one of `light`/`dark` must be present.
 */
export interface ThemeFamily {
  id: string;
  name: string;
  /** Which variant to use when the user picks "Auto" and no OS hint applies. */
  defaultVariant: 'light' | 'dark';
  light?: Theme;
  dark?: Theme;
}
