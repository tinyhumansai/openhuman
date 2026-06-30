/**
 * Theme token registry — the canonical list of themeable CSS variables that the
 * Theme Studio exposes and that {@link Theme} objects may override.
 *
 * Values live in `app/src/styles/tokens.css` (the Light `:root` / Dark
 * `:root.dark` defaults). A theme only carries the subset of tokens it changes;
 * ThemeProvider writes those as inline `--<key>` vars on <html>, and anything a
 * theme does not specify falls through to the tokens.css defaults. This keeps a
 * single source of truth for the base palette — we never duplicate every value
 * in TypeScript.
 */

/** Surface (background) token keys, without the leading `--`. */
export const SURFACE_KEYS = [
  'surface',
  'surface-canvas',
  'surface-muted',
  'surface-subtle',
  'surface-strong',
  'surface-hover',
  'surface-overlay',
] as const;

/** Text token keys. */
export const CONTENT_KEYS = [
  'content',
  'content-secondary',
  'content-muted',
  'content-faint',
  'content-inverted',
] as const;

/** Border token keys. */
export const LINE_KEYS = ['line', 'line-strong', 'line-subtle'] as const;

/** Accent palette family names. Each expands to shades 50…950. */
export const ACCENT_FAMILIES = ['primary', 'sage', 'amber', 'coral'] as const;

/** Tailwind accent shade steps. */
export const ACCENT_SHADES = [50, 100, 200, 300, 400, 500, 600, 700, 800, 900, 950] as const;

/** All accent token keys, e.g. `primary-500`. */
export const ACCENT_KEYS = ACCENT_FAMILIES.flatMap(fam =>
  ACCENT_SHADES.map(shade => `${fam}-${shade}` as const)
);

/** The `-500` base key of each accent family (what the editor shows by default). */
export const ACCENT_BASE_KEYS = ACCENT_FAMILIES.map(fam => `${fam}-500`);

/** Every colour token key the theme system knows about. */
export const ALL_COLOR_KEYS: string[] = [
  ...SURFACE_KEYS,
  ...CONTENT_KEYS,
  ...LINE_KEYS,
  ...ACCENT_KEYS,
];

export type ColorTokenKey = string;

/** Font role keys, mapped to the `--font-<role>` vars. */
export const FONT_ROLES = ['title', 'heading', 'body', 'mono', 'serif'] as const;
export type FontRole = (typeof FONT_ROLES)[number];

/**
 * UI grouping for the Theme Studio colour editor. `i18nKey` resolves the group
 * heading; `keys` are the tokens shown in that group. Accent shades beyond the
 * base `-500` are surfaced via the per-family advanced expander, not listed here.
 */
export interface ColorGroup {
  id: string;
  i18nKey: string;
  keys: string[];
}

export const COLOR_GROUPS: ColorGroup[] = [
  { id: 'surfaces', i18nKey: 'settings.theme.group.surfaces', keys: [...SURFACE_KEYS] },
  { id: 'text', i18nKey: 'settings.theme.group.text', keys: [...CONTENT_KEYS] },
  { id: 'borders', i18nKey: 'settings.theme.group.borders', keys: [...LINE_KEYS] },
  { id: 'accents', i18nKey: 'settings.theme.group.accents', keys: ACCENT_BASE_KEYS },
];

/** Curated font choices offered per role in the Theme Studio. */
export interface FontChoice {
  /** Stable id stored in the theme. */
  id: string;
  /** Human label (not translated — font names are proper nouns). */
  label: string;
  /** The CSS font-family stack written to the `--font-<role>` var. */
  stack: string;
}

const SANS_FALLBACK = `-apple-system, BlinkMacSystemFont, 'Segoe UI', Helvetica, Arial, sans-serif`;
const SERIF_FALLBACK = `Georgia, Cambria, 'Times New Roman', Times, serif`;
const MONO_FALLBACK = `'SF Mono', Consolas, 'Liberation Mono', Courier, monospace`;

export const FONT_CHOICES: FontChoice[] = [
  { id: 'inter', label: 'Inter', stack: `'Inter', ${SANS_FALLBACK}` },
  {
    id: 'cabinet',
    label: 'Cabinet Grotesk',
    stack: `'Cabinet Grotesk', 'Inter', ${SANS_FALLBACK}`,
  },
  { id: 'system', label: 'System UI', stack: `system-ui, ${SANS_FALLBACK}` },
  { id: 'newsreader', label: 'Newsreader (serif)', stack: `'Newsreader', ${SERIF_FALLBACK}` },
  { id: 'georgia', label: 'Georgia (serif)', stack: SERIF_FALLBACK },
  { id: 'jetbrains', label: 'JetBrains Mono', stack: `'JetBrains Mono', ${MONO_FALLBACK}` },
];

/** Find a {@link FontChoice} whose stack matches a stored value (best-effort). */
export function fontChoiceForStack(stack: string | undefined): FontChoice | undefined {
  if (!stack) return undefined;
  return FONT_CHOICES.find(c => c.stack === stack);
}
