import type { Theme, ThemeFamily } from './types';

/**
 * Built-in theme presets, grouped into families. Each family exposes a Light
 * and/or Dark variant; the Theme Studio picks the family and a Light/Dark/Auto
 * variant. `classic.light` / `classic.dark` carry no overrides — they rely on
 * the tokens.css defaults so they always match the historical palette.
 *
 * Variant ids are stable (`light`, `dark`, `ocean`, `ocean-dark`, …) so existing
 * persisted selections keep resolving.
 */

export const LIGHT_THEME_ID = 'light';
export const DARK_THEME_ID = 'dark';

const MONO_STACK = `'JetBrains Mono', 'SF Mono', Consolas, 'Liberation Mono', Courier, monospace`;
const SERIF_STACK = `'Newsreader', Georgia, Cambria, 'Times New Roman', Times, serif`;

// Full primary ramps (50…950) for accent-recoloured themes. Components use many
// shades via `dark:text-primary-300` etc., so a theme that only overrode 500-700
// would leave the others showing the default blue. Override the whole ramp.
const GREEN_RAMP: Record<string, string> = {
  'primary-50': '230 255 240',
  'primary-100': '198 250 218',
  'primary-200': '150 240 185',
  'primary-300': '90 226 146',
  'primary-400': '30 212 110',
  'primary-500': '0 230 90',
  'primary-600': '0 196 76',
  'primary-700': '0 164 64',
  'primary-800': '0 122 50',
  'primary-900': '0 92 40',
  'primary-950': '0 52 23',
};
const RED_RAMP: Record<string, string> = {
  'primary-50': '254 235 235',
  'primary-100': '252 214 214',
  'primary-200': '248 178 178',
  'primary-300': '240 130 130',
  'primary-400': '233 84 84',
  'primary-500': '230 40 40',
  'primary-600': '200 30 30',
  'primary-700': '170 24 24',
  'primary-800': '140 20 20',
  'primary-900': '112 18 18',
  'primary-950': '70 10 10',
};
const BROWN_RAMP: Record<string, string> = {
  'primary-50': '246 236 222',
  'primary-100': '238 222 200',
  'primary-200': '226 198 160',
  'primary-300': '210 170 118',
  'primary-400': '194 144 86',
  'primary-500': '180 120 60',
  'primary-600': '156 100 46',
  'primary-700': '130 82 38',
  'primary-800': '104 64 30',
  'primary-900': '82 50 24',
  'primary-950': '48 30 15',
};

// ── Classic ────────────────────────────────────────────────────────────────
const CLASSIC_LIGHT: Theme = {
  id: LIGHT_THEME_ID,
  name: 'Light',
  isDark: false,
  builtIn: true,
  colors: {},
  fonts: {},
};
const CLASSIC_DARK: Theme = {
  id: DARK_THEME_ID,
  name: 'Dark',
  isDark: true,
  builtIn: true,
  colors: {},
  fonts: {},
};

// ── Ocean ──────────────────────────────────────────────────────────────────
const OCEAN_LIGHT: Theme = {
  id: 'ocean',
  name: 'Ocean Light',
  isDark: false,
  builtIn: true,
  colors: {
    'surface-canvas': '233 242 252',
    surface: '255 255 255',
    'surface-muted': '224 236 248',
    'surface-subtle': '230 240 250',
    'surface-hover': '214 230 248',
    line: '203 222 242',
    'line-subtle': '224 236 248',
    content: '15 36 56',
    'content-secondary': '45 70 96',
    'primary-500': '74 131 221',
    'primary-600': '53 110 200',
    'primary-700': '40 92 176',
  },
  gradient: { canvas: 'linear-gradient(180deg, rgb(235 244 253), rgb(214 230 248))' },
  fonts: {},
};
const OCEAN_DARK: Theme = {
  id: 'ocean-dark',
  name: 'Ocean Dark',
  isDark: true,
  builtIn: true,
  colors: {
    'surface-canvas': '7 12 24',
    surface: '22 30 52',
    'surface-muted': '30 40 66',
    'surface-subtle': '27 36 60',
    'surface-strong': '40 52 84',
    'surface-hover': '40 52 84',
    'surface-overlay': '0 0 0',
    line: '48 62 98',
    'line-strong': '70 88 130',
    'line-subtle': '36 48 76',
    content: '224 232 244',
    'content-secondary': '182 196 220',
    'content-muted': '140 156 188',
    'content-faint': '104 118 150',
    'primary-500': '96 165 250',
    'primary-600': '59 130 246',
  },
  gradient: { canvas: 'radial-gradient(circle at 30% 0%, rgb(20 34 64), rgb(7 12 24) 60%)' },
  fonts: {},
};

// ── Sepia ──────────────────────────────────────────────────────────────────
const SEPIA_LIGHT: Theme = {
  id: 'sepia',
  name: 'Sepia Light',
  isDark: false,
  builtIn: true,
  colors: {
    'surface-canvas': '244 236 222',
    surface: '250 244 233',
    'surface-muted': '238 228 210',
    'surface-subtle': '240 231 215',
    'surface-strong': '232 220 198',
    'surface-hover': '232 220 198',
    line: '222 209 186',
    'line-strong': '206 190 162',
    'line-subtle': '234 224 206',
    content: '60 50 38',
    'content-secondary': '90 76 58',
    'content-muted': '120 104 82',
    'content-faint': '160 144 120',
    ...BROWN_RAMP,
  },
  fonts: { body: SERIF_STACK, heading: SERIF_STACK },
};
const SEPIA_DARK: Theme = {
  id: 'sepia-dark',
  name: 'Sepia Dark',
  isDark: true,
  builtIn: true,
  colors: {
    'surface-canvas': '26 22 17',
    surface: '40 34 26',
    'surface-muted': '48 41 31',
    'surface-subtle': '44 37 28',
    'surface-strong': '58 49 37',
    'surface-hover': '56 47 35',
    'surface-overlay': '0 0 0',
    line: '64 54 41',
    'line-strong': '90 76 58',
    'line-subtle': '48 40 30',
    content: '236 228 214',
    'content-secondary': '198 184 162',
    'content-muted': '156 142 120',
    'content-faint': '120 106 86',
    ...BROWN_RAMP,
    'primary-500': '200 150 90',
    'primary-600': '176 126 70',
  },
  fonts: { body: SERIF_STACK, heading: SERIF_STACK },
};

// ── Matrix (terminal green) ─────────────────────────────────────────────────
const MATRIX_DARK: Theme = {
  id: 'matrix',
  name: 'Matrix',
  isDark: true,
  builtIn: true,
  colors: {
    'surface-canvas': '2 8 4',
    surface: '6 18 10',
    'surface-muted': '10 26 14',
    'surface-subtle': '8 22 12',
    'surface-strong': '14 36 20',
    'surface-hover': '16 42 24',
    'surface-overlay': '0 0 0',
    line: '22 64 34',
    'line-strong': '36 104 54',
    'line-subtle': '14 44 24',
    content: '134 255 168',
    'content-secondary': '78 210 122',
    'content-muted': '58 158 92',
    'content-faint': '44 112 68',
    'content-inverted': '2 8 4',
    ...GREEN_RAMP,
  },
  gradient: { canvas: 'radial-gradient(circle at 50% 0%, rgb(6 32 16), rgb(2 8 4) 68%)' },
  fonts: { body: MONO_STACK, heading: MONO_STACK },
};
const MATRIX_LIGHT: Theme = {
  id: 'matrix-light',
  name: 'Matrix Light',
  isDark: false,
  builtIn: true,
  colors: {
    'surface-canvas': '234 245 237',
    surface: '246 252 248',
    'surface-muted': '224 240 229',
    'surface-subtle': '230 244 234',
    'surface-hover': '214 236 222',
    line: '198 226 206',
    'line-strong': '170 206 180',
    'line-subtle': '224 240 229',
    content: '10 60 28',
    'content-secondary': '30 92 50',
    'content-muted': '60 122 80',
    'content-faint': '110 160 124',
    ...GREEN_RAMP,
    'primary-500': '0 160 60',
    'primary-600': '0 132 50',
  },
  fonts: { body: MONO_STACK, heading: MONO_STACK },
};

// ── HAL 9000 (red eye) ──────────────────────────────────────────────────────
const HAL_DARK: Theme = {
  id: 'hal9000',
  name: 'HAL 9000',
  isDark: true,
  builtIn: true,
  colors: {
    'surface-canvas': '8 4 4',
    surface: '20 12 12',
    'surface-muted': '28 16 16',
    'surface-subtle': '24 14 14',
    'surface-strong': '38 20 20',
    'surface-hover': '40 22 22',
    'surface-overlay': '0 0 0',
    line: '60 28 28',
    'line-strong': '96 40 40',
    'line-subtle': '40 20 20',
    content: '240 224 224',
    'content-secondary': '210 180 180',
    'content-muted': '170 130 130',
    'content-faint': '130 96 96',
    ...RED_RAMP,
  },
  gradient: { canvas: 'radial-gradient(circle at 50% 16%, rgb(84 10 10), rgb(8 4 4) 56%)' },
  fonts: {},
};
const HAL_LIGHT: Theme = {
  id: 'hal9000-light',
  name: 'HAL 9000 Light',
  isDark: false,
  builtIn: true,
  colors: {
    'surface-canvas': '245 238 238',
    surface: '252 247 247',
    'surface-muted': '240 228 228',
    'surface-subtle': '244 234 234',
    'surface-hover': '236 222 222',
    line: '226 200 200',
    'line-strong': '206 174 174',
    'line-subtle': '240 228 228',
    content: '60 24 24',
    'content-secondary': '100 50 50',
    'content-muted': '140 90 90',
    'content-faint': '170 130 130',
    ...RED_RAMP,
    'primary-500': '210 40 40',
    'primary-600': '180 30 30',
  },
  fonts: {},
};

export const THEME_FAMILIES: ThemeFamily[] = [
  {
    id: 'classic',
    name: 'Classic',
    defaultVariant: 'light',
    light: CLASSIC_LIGHT,
    dark: CLASSIC_DARK,
  },
  { id: 'ocean', name: 'Ocean', defaultVariant: 'light', light: OCEAN_LIGHT, dark: OCEAN_DARK },
  { id: 'sepia', name: 'Sepia', defaultVariant: 'light', light: SEPIA_LIGHT, dark: SEPIA_DARK },
  { id: 'matrix', name: 'Matrix', defaultVariant: 'dark', light: MATRIX_LIGHT, dark: MATRIX_DARK },
  { id: 'hal9000', name: 'HAL 9000', defaultVariant: 'dark', light: HAL_LIGHT, dark: HAL_DARK },
];

/** Flat list of every concrete variant — for id lookup and back-compat. */
export const PRESET_THEMES: Theme[] = THEME_FAMILIES.flatMap(f =>
  [f.light, f.dark].filter((t): t is Theme => Boolean(t))
);

export function findPreset(id: string): Theme | undefined {
  return PRESET_THEMES.find(t => t.id === id);
}

export function findFamily(id: string): ThemeFamily | undefined {
  return THEME_FAMILIES.find(f => f.id === id);
}

/** The family that owns a given variant theme id (e.g. `ocean-dark` → ocean). */
export function familyForThemeId(id: string): ThemeFamily | undefined {
  return THEME_FAMILIES.find(f => f.light?.id === id || f.dark?.id === id);
}

/**
 * Resolve a family + desired variant to a concrete Theme. Falls back to the
 * family's available variant when the requested one is missing.
 */
export function resolveFamilyVariant(family: ThemeFamily, variant: 'light' | 'dark'): Theme {
  const chosen = variant === 'dark' ? family.dark : family.light;
  return chosen ?? family.dark ?? family.light ?? CLASSIC_LIGHT;
}
