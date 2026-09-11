# Theming

OpenHuman is fully re-skinnable at runtime. Colours and fonts are driven by CSS
variables (the "tokens"), so a theme is just a set of values for those variables.
This page is the contributor reference for the token system.

## How it works

1. **Tokens**: `app/src/styles/tokens.css` defines every themeable colour as a
   space-separated **RGB channel triple** (e.g. `--surface: 255 255 255;`) plus
   font-role vars (`--font-title/heading/body/mono/serif`). The Light palette
   lives in `:root`; the Dark palette in `:root.dark`.

2. **Tailwind wiring**: `app/tailwind.config.js` exposes the tokens as utility
   colours via `rgb(var(--token) / <alpha-value>)`. The `<alpha-value>` form is
   what keeps opacity modifiers working (`bg-surface/50`, `bg-primary-500/10`).
   Channel format is mandatory for this reason: never store a token as a hex
   string.

3. **Runtime application**: `app/src/providers/ThemeProvider.tsx` resolves the
   active `Theme` and writes its overrides as inline `--token` / `--font-<role>`
   variables on `<html>`, toggling `.dark` from `theme.isDark`. Variables a theme
   doesn't override fall through to the tokens.css defaults; variables left over
   from a previous theme are removed on switch.

4. **State**: `app/src/store/themeSlice.ts` holds `activeThemeId` and
   `customThemes`. Built-in presets live in `app/src/lib/theme/presets.ts`.
   Users edit themes in **Settings → Theme Studio**
   (`app/src/components/settings/panels/ThemeStudioPanel.tsx`).

## Token taxonomy

| Group    | Tokens                                                                                                               | Tailwind utilities                                                             |
| -------- | -------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------ |
| Surfaces | `surface`, `surface-canvas`, `surface-muted`, `surface-subtle`, `surface-strong`, `surface-hover`, `surface-overlay` | `bg-surface`, `bg-surface-muted`, …                                            |
| Text     | `content`, `content-secondary`, `content-muted`, `content-faint`, `content-inverted`                                 | `text-content`, `text-content-muted`, …                                        |
| Borders  | `line`, `line-strong`, `line-subtle`                                                                                 | `border-line`, `border-line-strong`, …                                         |
| Accents  | `primary-*`, `sage-*`, `amber-*`, `coral-*` (shades 50…950)                                                          | `bg-primary-500`, `text-coral-600`, … (var-backed, themeable, unchanged names) |
| Fonts    | `font-title`, `font-heading`, `font-body`, `font-mono`, `font-serif`                                                 | `font-title`, `font-heading`, `font-body`, …                                   |

The legacy `--cmd-*` and `--color-*` variable sets are thin aliases over these
canonical tokens. Don't add new colours there.

## Authoring components

- Use semantic utilities (`bg-surface`, `text-content`, `border-line`) for
  neutral surfaces/text/borders instead of `bg-white dark:bg-neutral-900` etc.
  You almost never need `dark:` variants for these, because the token flips for you.
- Use the accent palettes (`primary`/`sage`/`amber`/`coral`) for semantic colour;
  they're themeable with no extra work.
- Avoid hardcoded hex in `className` or inline `style`, since those bypass theming.

## Colour as identity: the four-ramp ceiling

A recurring shape in this codebase is a lookup table that answers "which thing
is this?" with a colour — a skill category, an event-log domain, a notification
provider, a catalogue source. Those tables are where stock Tailwind ramps keep
creeping back in, because a table with nine rows wants nine hues and the app
ships four.

**There are exactly four themeable ramps: `primary`, `sage`, `amber`, `coral`.**
Everything else in Tailwind's default palette (`emerald`, `violet`, `sky`,
`teal`, `indigo`, `cyan`, `rose`, `pink`, `purple`, …) resolves to a fixed oklch
value that ignores the user's active theme entirely. A table built on those hues
looks fine in the default skin and falls apart in every other one.

### The rule

1. **Map a stock ramp to its themeable equivalent at the same shade step:**
   `red → coral`, `green`/`emerald` → `sage`, `orange → amber`, `blue → primary`.
   `bg-emerald-50 text-emerald-700` becomes `bg-sage-50 text-sage-700`.

2. **Hues that have no equivalent do not get one.** `violet`, `teal`, `sky`,
   `cyan`, `indigo`, `pink` and `purple` are not "nearly primary" or "nearly
   sage". Do not invent a fifth ramp, do not duplicate an existing one under a
   new name, and do not reach for `--accent-lavender` and friends — those are
   fixed hexes, not ramps.

3. **When a table needs more than four distinct hues, send the surplus rows to
   the neutral pair the table already defines** (`bg-surface-subtle
text-content-secondary`, or whatever that table's "unknown"/"other" row
   uses). Never let two rows collide on the same ramp: two domains rendering
   identically destroys the exact distinction the table exists to encode, which
   is strictly worse than rendering one of them in neutral.

4. **Decide which rows keep a hue by which distinction a reader acts on.** The
   badge almost always prints its own label, so colour is a scanning aid, not
   the information itself. Spend the four ramps on the readings that change what
   someone does, and let the rest go neutral. Keep the semantics honest while
   you are at it: `coral` reads as failure, so an ordinary row painted coral
   makes routine state look broken. Leaving a ramp unassigned is a legitimate
   outcome.

Worked examples in the tree:

| Table                                                     | Rows            | Kept a hue                                                                                  | Why                                                                                                                                           |
| --------------------------------------------------------- | --------------- | ------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------- |
| `skills/skillIcons.tsx` `CATEGORY_META`                   | 9               | `Built-in` (primary), `Productivity` (sage), `Social` (coral), `Tools & Automation` (amber) | `Channels`, `Chat` and `Platform` share the neutral tone of `All` / `Other`                                                                   |
| `skills/SkillsExplorerTab.tsx` `SOURCE_COLORS`            | 6               | `built-in` (sage), `optional` (primary)                                                     | The four remote catalogues print their own name; provenance tier is the distinction that matters                                              |
| `skills/SkillsExplorerTab.tsx` `FORMAT_MAP`               | 5 rows, 3 tones | Hermes family (primary), ClawHub family (sage), `legacy` (amber)                            | Three tones fit under the ceiling, so nothing is lost                                                                                         |
| `settings/panels/EventLogPanel.tsx` `DOMAIN_BADGE_COLORS` | 11              | `tool` (primary), `agent` (sage), `approval` (amber)                                        | Who acted, and what waits on a human. Coral stays unassigned — no domain means failure                                                        |
| `notifications/NotificationCard.tsx` provider badge       | 6               | none                                                                                        | The importance badge in the same row already spends coral/amber/sage on high/medium/low; a coral provider would read as a failed notification |

### Brand tints are a separate question

A few plates are a third party's brand colour, not an app hue — Telegram's
`#249CD8`, Discord's `#5865F2`, iMessage's `#34C759` in
`skills/skillIcons.tsx`. Flattening those to `bg-surface-subtle` erases them
into the generic badge beside them, so they are deliberately left as hex.
Giving them a themeable home means **adding brand tokens**, which is a product
decision rather than a cleanup. The same applies to the provider badges in
`NotificationCard.tsx`: reaching back for a stock ramp is not the fix.

### Do not repaint a primitive's variant

`<Button variant="primary" className="bg-violet-500">` is the same bug wearing a
different hat: the variant already paints the accent ramp, and the override
both freezes the colour and desynchronises hover, focus and disabled states.
Retint the surface around it instead, and drop the override.

## The migration codemod

`scripts/theme-codemod/` collapses audited `light dark:` Tailwind pairings into
the semantic utilities. It is idempotent and dry-run by default:

```bash
node scripts/theme-codemod/migrate.mjs            # dry-run + report
node scripts/theme-codemod/migrate.mjs --write    # apply
node scripts/theme-codemod/migrate.mjs --selftest # fixture assertions
```

It only rewrites adjacent pairs and never touches opacity-suffixed utilities or
test files. Mapping table: `scripts/theme-codemod/map.mjs`.
