#!/usr/bin/env node
/**
 * lint:ui-tokens companion — fail on Tailwind utilities that name a scale or
 * token the theme never defines.
 *
 * Motivation: `ocean-*` shipped across ~26 component files and emitted ZERO
 * CSS, because `ocean` is not a colour in `tailwind.config.js` (it is only a
 * *theme preset id* in `src/lib/theme/presets.ts`). Those elements rendered
 * with no background, no text colour and no border. The old rg-based
 * `lint:ui-tokens` regex only banned scales that DO exist but are off-palette
 * (`neutral|stone|slate|canvas|white|black`), so an undefined scale — the more
 * damaging case, since it is silently invisible — slipped straight through.
 *
 * This script inverts the check: instead of a hand-maintained deny-list it
 * derives the ALLOWED names from the theme itself (`src/index.css`'s `@theme`
 * block plus Tailwind's own default theme) and fails on anything else.
 *
 * Two passes, because two different shapes of dead utility exist:
 *
 *  1. SHADED — `<utility>-<scale>-<shade>` whose `<scale>-<shade>` pair the
 *     theme does not define (`bg-ocean-500`).
 *  2. SHADELESS — `<utility>-<name>` (optionally `/<alpha>`) whose `<name>` is
 *     neither a bare colour (`--color-<name>`), a non-colour keyword of that
 *     utility (`text-center`, `border-dashed`), nor a named scale the utility
 *     reads (`--shadow-*` for `shadow-`, `--text-*` for `text-`). This pass is
 *     what catches `text-danger`, `text-ink`, `text-coral` / `bg-coral/20`
 *     (`coral` is defined only WITH shade steps), `bg-surface-secondary`,
 *     `text-md` and `shadow-strong` — every one of which renders with no
 *     colour, no size or no elevation at all.
 *
 * Deliberately NOT flagged:
 *  - bare words (`ocean` as a preset id, a CSS custom property `--ocean`, a
 *    comment) — only utility-shaped matches count;
 *  - arbitrary values (`bg-[#D97757]`, `transition-[border-color]`), which
 *    need no scale — bracket groups are stripped before matching;
 *  - non-shade numeric suffixes (`border-l-2`, `w-12`, `divide-y-0`), because
 *    the shaded pass requires a real Tailwind shade step and the shadeless
 *    pass requires every name segment to begin with a letter;
 *  - prose. The shadeless pass skips `src/lib/i18n/` and test files, where
 *    "text-to-speech", "to-do" and fixture strings like `bg-noise` are English
 *    and test data, not class lists. The shaded pass still scans them.
 */
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { createRequire } from 'node:module';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import tailwindColorsModule from 'tailwindcss/colors';

const here = path.dirname(fileURLToPath(import.meta.url));
const appRoot = path.resolve(here, '..');

const SHADES = new Set([
  '50',
  '100',
  '150',
  '200',
  '300',
  '400',
  '500',
  '600',
  '700',
  '800',
  '900',
  '950',
]);

/** Colour-bearing utility prefixes, including directional border/divide forms. */
const UTILITY_PREFIXES = [
  'bg',
  'text',
  'border',
  'border-x',
  'border-y',
  'border-t',
  'border-r',
  'border-b',
  'border-l',
  'ring',
  'ring-offset',
  'divide',
  'divide-x',
  'divide-y',
  'outline',
  'shadow',
  'fill',
  'stroke',
  'accent',
  'caret',
  'decoration',
  'placeholder',
  'from',
  'to',
  'via',
];

/**
 * Non-colour names each utility legitimately accepts. Without these the
 * shadeless pass would flag `text-center`, `border-dashed` and friends, none
 * of which is a colour at all. Keyed by the exact prefix that matched, so a
 * directional form (`border-t-`) only ever accepts a colour.
 */
const NON_COLOR_NAMES = {
  bg: [
    'auto',
    'bottom',
    'center',
    'contain',
    'cover',
    'fixed',
    'left',
    'local',
    'none',
    'right',
    'scroll',
    'top',
  ],
  text: [
    'balance',
    'center',
    'clip',
    'ellipsis',
    'end',
    'justify',
    'left',
    'nowrap',
    'pretty',
    'right',
    'start',
    'wrap',
  ],
  border: [
    'b',
    'collapse',
    'dashed',
    'dotted',
    'double',
    'e',
    'hidden',
    'l',
    'none',
    'r',
    's',
    'separate',
    'solid',
    't',
    'x',
    'y',
  ],
  divide: [
    'dashed',
    'dotted',
    'double',
    'hidden',
    'none',
    'solid',
    'x',
    'x-reverse',
    'y',
    'y-reverse',
  ],
  'divide-x': ['reverse'],
  'divide-y': ['reverse'],
  outline: ['dashed', 'dotted', 'double', 'hidden', 'none', 'solid'],
  ring: ['inset'],
  shadow: ['none'],
  fill: ['none'],
  stroke: ['none'],
  accent: ['auto'],
  caret: [],
  decoration: [
    'auto',
    'clone',
    'dashed',
    'dotted',
    'double',
    'from-font',
    'none',
    'slice',
    'solid',
    'wavy',
  ],
  placeholder: [],
};

/**
 * Name FAMILIES (matched by leading segment) each utility accepts. These are
 * the multi-part non-colour utilities — `bg-linear-to-br`, `bg-clip-text`,
 * `fill-mode-forwards` (tw-animate-css) — where enumerating every member would
 * rot. A family entry allows `<prefix>-<family>` and `<prefix>-<family>-…`.
 */
const NON_COLOR_FAMILIES = {
  bg: [
    'blend',
    'clip',
    'conic',
    'gradient',
    'linear',
    'origin',
    'position',
    'radial',
    'repeat',
    'no-repeat',
    'size',
  ],
  text: ['shadow'],
  border: ['spacing'],
  fill: ['mode'],
  decoration: [],
};

/**
 * Hyphenated identifiers that are NOT class names but do look like one to the
 * regex, in files the shadeless pass still scans. Keep this list short and
 * justified — every entry is a hole in the check.
 *
 *  - `bg-image`   — a tailwind-merge class-group key in `src/lib/cn.ts`.
 *  - `stroke-*`   — SVG presentation attributes inside the inline data-URI
 *                   chevron in `src/components/ui/NativeSelect.tsx`.
 */
const NON_UTILITY_IDENTIFIERS = new Set([
  'bg-image',
  'stroke-linecap',
  'stroke-linejoin',
  'stroke-width',
]);

/**
 * Tailwind v4 removed `resolveConfig` and this app now defines its custom
 * palette in `src/index.css`'s `@theme` block. Build the effective shade set
 * from Tailwind's exported default palette plus every numeric
 * `--color-<scale>-<shade>` variable declared by the app, and the bare-name
 * sets (`--color-<name>`, `--shadow-<name>`, `--text-<name>`) alongside them.
 */
function shadeResolver() {
  const colors = tailwindColorsModule.default ?? tailwindColorsModule;
  const shades = new Set();
  const scaleNames = new Set();
  /** Colours usable with no shade step at all: `bg-white`, `text-content`. */
  const bareColors = new Set();
  /** Named scales `shadow-<name>` reads. */
  const shadowNames = new Set(['none']);
  /** Named scales `text-<name>` reads for font size. */
  const textScaleNames = new Set();
  /** Named scales `bg-<name>` reads for background images. */
  const backgroundImageNames = new Set();

  for (const [scale, values] of Object.entries(colors)) {
    if (typeof values === 'string') {
      // `inherit` / `current` / `transparent` / `black` / `white`.
      bareColors.add(scale);
      continue;
    }
    if (!values || typeof values !== 'object') continue;
    for (const shade of Object.keys(values)) {
      if (!/^\d+$/.test(shade)) continue;
      shades.add(`${scale}-${shade}`);
      scaleNames.add(scale);
    }
  }

  /** Fold one `@theme`-shaped stylesheet into the sets above. */
  const absorbThemeCss = css => {
    for (const [, name] of css.matchAll(/--color-([a-z][a-z0-9-]*)\s*:/g)) {
      const shaded = name.match(/^(.*)-(\d{1,3})$/);
      if (shaded) {
        shades.add(name);
        scaleNames.add(shaded[1]);
      } else {
        bareColors.add(name);
      }
    }
    for (const [, name] of css.matchAll(/--shadow-([a-z][a-z0-9-]*)\s*:/g)) shadowNames.add(name);
    for (const [, name] of css.matchAll(/--text-([a-z][a-z0-9-]*)\s*:/g)) {
      // Skip the paired `--text-<size>--line-height` / `--letter-spacing` keys.
      if (name.includes('--')) continue;
      textScaleNames.add(name);
    }
    for (const [, name] of css.matchAll(/--background-image-([a-z][a-z0-9-]*)\s*:/g)) {
      backgroundImageNames.add(name);
    }
  };

  absorbThemeCss(readFileSync(path.join(appRoot, 'src/index.css'), 'utf8'));

  // Tailwind's own default theme, for the names the app does not redeclare
  // (`shadow-md`, `text-2xl`, `text-shadow-sm`, …). Read from the installed
  // package so a Tailwind upgrade cannot silently strand this lint.
  try {
    const require = createRequire(import.meta.url);
    const tailwindRoot = path.dirname(require.resolve('tailwindcss/package.json'));
    absorbThemeCss(readFileSync(path.join(tailwindRoot, 'theme.css'), 'utf8'));
  } catch {
    // Layout changed upstream; fall back to the v4 defaults this app relies on
    // rather than reporting every `shadow-md` in the tree as undefined.
    for (const n of ['2xs', 'xs', 'sm', 'md', 'lg', 'xl', '2xl', 'inner']) shadowNames.add(n);
    for (const n of ['xs', 'sm', 'base', 'lg', 'xl']) textScaleNames.add(n);
  }

  if (!shades.has('primary-500')) {
    throw new Error(
      'lint:ui-tokens: Tailwind v4 theme has no primary-500 — refusing to run, ' +
        'because the app palette could not be loaded.'
    );
  }

  return {
    shadeExists: (scale, shade) => shades.has(`${scale}-${shade}`),
    scaleNames: [...scaleNames],
    bareColors,
    shadowNames,
    textScaleNames,
    backgroundImageNames,
    /** Scales that exist ONLY with a shade step — `text-coral` emits nothing. */
    shadedOnlyScales: new Set([...scaleNames].filter(s => !bareColors.has(s))),
  };
}

const PATTERN = new RegExp(
  `\\b(${UTILITY_PREFIXES.join('|')})-([a-z][a-z0-9]+)-(\\d{2,3})\\b`,
  'g'
);

/**
 * Shadeless form. Longest prefix first so `border-t-line` reads as
 * `border-t` + `line`, not `border` + `t-line`. Every name segment must begin
 * with a letter, which is what keeps `border-b-2`, `bg-coral-500` and
 * `text-embedding-3-small` out of this pass — the first two belong to the
 * shaded pass and the third is a model id, not a class.
 */
const SHADELESS_PATTERN = new RegExp(
  `(?<![-\\w])(${[...UTILITY_PREFIXES].sort((a, b) => b.length - a.length).join('|')})` +
    `-([a-z][a-z0-9]*(?:-[a-z][a-z0-9]*)*)(?:\\/(?:\\d{1,3}|\\[[^\\]]+\\]))?(?![\\w-])`,
  'g'
);

/** Tailwind arbitrary values / variant selectors — never a scale reference. */
const ARBITRARY_VALUE = /(?<=[-:])\[[^\]]*\]/g;

function* walk(dir) {
  for (const entry of readdirSync(dir)) {
    if (entry === 'node_modules' || entry === 'dist' || entry.startsWith('.')) continue;
    const full = path.join(dir, entry);
    if (statSync(full).isDirectory()) {
      yield* walk(full);
    } else if (/\.(ts|tsx|js|jsx)$/.test(entry)) {
      yield full;
    }
  }
}

/**
 * Translation catalogues and test fixtures hold English and sample data, not
 * class lists — "text-to-speech", "to-do", `bg-noise`. Only the shadeless pass
 * is loose enough to trip on those, so only it skips them.
 */
function holdsProseNotClasses(relative) {
  // `path.relative` yields the platform separator, so on Windows these paths
  // arrive as `src\\lib\\i18n\\en.ts` and every `/`-spelled test below misses.
  // The skip then never fires and the catalogues are scanned as class lists:
  // "text-to-speech" is read as `text-` + an undefined token.
  const posix = relative.split(path.sep).join('/');

  return (
    posix.startsWith('src/lib/i18n/') ||
    posix.includes('/__tests__/') ||
    /\.(test|spec)\.(ts|tsx|js|jsx)$/.test(posix)
  );
}

const {
  shadeExists,
  scaleNames,
  bareColors,
  shadowNames,
  textScaleNames,
  backgroundImageNames,
  shadedOnlyScales,
} = shadeResolver();

/**
 * @returns {string | null} why `<prefix>-<name>` emits no CSS, or null if it is
 * a name the theme actually defines.
 */
function shadelessViolation(prefix, name) {
  if (NON_UTILITY_IDENTIFIERS.has(`${prefix}-${name}`)) return null;
  if (bareColors.has(name)) return null;
  if ((NON_COLOR_NAMES[prefix] ?? []).includes(name)) return null;
  const families = NON_COLOR_FAMILIES[prefix] ?? [];
  if (families.some(f => name === f || name.startsWith(`${f}-`))) return null;
  if (prefix === 'shadow' && shadowNames.has(name)) return null;
  if (prefix === 'text' && textScaleNames.has(name)) return null;
  if (prefix === 'bg' && backgroundImageNames.has(name)) return null;

  if (shadedOnlyScales.has(name)) {
    return `${name} is defined only with shade steps (use ${name}-500)`;
  }
  return `${name} is not a token this theme defines`;
}

const violations = [];

for (const file of walk(path.join(appRoot, 'src'))) {
  const relative = path.relative(appRoot, file);
  const scanShadeless = !holdsProseNotClasses(relative);
  const lines = readFileSync(file, 'utf8').split('\n');
  lines.forEach((line, i) => {
    // Skip comment-only lines — prose is allowed to name a retired scale.
    if (/^\s*(\/\/|\*|\/\*)/.test(line)) return;
    for (const m of line.matchAll(PATTERN)) {
      const [match, , scale, shade] = m;
      if (!SHADES.has(shade)) continue;
      if (shadeExists(scale, shade)) continue;
      violations.push(`${relative}:${i + 1}: ${match}`);
    }
    if (!scanShadeless) return;
    const stripped = line.replace(ARBITRARY_VALUE, '');
    for (const m of stripped.matchAll(SHADELESS_PATTERN)) {
      const [match, prefix, name] = m;
      const why = shadelessViolation(prefix, name);
      if (!why) continue;
      violations.push(`${relative}:${i + 1}: ${match}  (${why})`);
    }
  });
}

if (violations.length > 0) {
  console.error(
    `lint:ui-tokens: ${violations.length} Tailwind utility/utilities name a scale or token that ` +
      `the Tailwind v4 theme does not define. These emit NO CSS and render uncoloured:\n`
  );
  for (const v of violations) console.error(`  ${v}`);
  console.error(
    `\nScales that define numeric shades: ${[...scaleNames].sort().join(', ')}\n` +
      `Fix by choosing a defined semantic token — do NOT add a scale only to silence this lint.`
  );
  process.exit(1);
}

console.log(
  `lint:ui-tokens: no undefined colour scales or tokens (${scaleNames.length} shade-bearing scales, ` +
    `${bareColors.size} shadeless colours, ${shadowNames.size} shadows).`
);
