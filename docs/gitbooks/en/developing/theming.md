---
description: >-
  The CSS token system behind every theme: how tokens are defined, how
  Tailwind uses them and how to write components that follow it.
icon: palette
---

# Theming and design tokens

You can re-skin OpenHuman at runtime. CSS variables (the "tokens") drive colors and fonts, so a theme is a set of values for those variables. This page is the contributor reference for the token system.

## How it works

1. Tokens: `app/src/styles/tokens.css` defines every themeable color as a space-separated RGB channel triple (for example `--surface: 255 255 255;`), plus font-role variables (`--font-title/heading/body/mono/serif`). The light palette lives in `:root` and the dark palette in `:root.dark`.

2. Tailwind wiring: the app runs Tailwind v4, so there is no `tailwind.config.js`. The `@theme` block in `app/src/index.css` exposes each token as a utility color (`--color-surface: rgb(var(--surface));` and so on). Tailwind's opacity modifiers then handle `bg-surface/50` and `bg-primary-500/10` with no extra work. This only works because tokens are RGB triples. A hex string cannot be combined with an opacity modifier this way.

3. Runtime application: `app/src/providers/ThemeProvider.tsx` resolves the active `Theme` and writes its overrides as inline `--token` and `--font-<role>` variables on `<html>`. It toggles `.dark` from `theme.isDark`. Variables a theme does not override fall back to the `tokens.css` defaults. Variables left over from a previous theme are removed on switch.

4. State: `app/src/store/themeSlice.ts` holds `activeThemeId` and `customThemes`. Built-in presets live in `app/src/lib/theme/presets.ts`. Users edit themes in Settings > Theme Studio (`app/src/components/settings/panels/ThemeStudioPanel.tsx`).

## Token taxonomy

| Group    | Tokens                                                                                                               | Tailwind utilities                                                             |
| -------- | -------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------ |
| Surfaces | `surface`, `surface-canvas`, `surface-muted`, `surface-subtle`, `surface-strong`, `surface-hover`, `surface-overlay` | `bg-surface`, `bg-surface-muted`, ...                                            |
| Text     | `content`, `content-secondary`, `content-muted`, `content-faint`, `content-inverted`                                 | `text-content`, `text-content-muted`, ...                                        |
| Borders  | `line`, `line-strong`, `line-subtle`                                                                                 | `border-line`, `border-line-strong`, ...                                         |
| Accents  | `primary-*`, `sage-*`, `amber-*`, `coral-*` (shades 50 to 950)                                                          | `bg-primary-500`, `text-coral-600`, ... (backed by variables, themeable) |
| Fonts    | `font-title`, `font-heading`, `font-body`, `font-mono`, `font-serif`                                                 | `font-title`, `font-heading`, `font-body`, ...                                   |

The legacy `--cmd-*` and `--color-*` variable sets are thin aliases over these tokens. Do not add new colors there.

## Authoring components

- Use semantic utilities (`bg-surface`, `text-content`, `border-line`) for neutral surfaces, text and borders instead of `bg-white dark:bg-neutral-900`. You rarely need `dark:` variants, because the token flips for you.
- Use the accent palettes (`primary`, `sage`, `amber`, `coral`) for semantic color. They are themeable with no extra work.
- Avoid hardcoded hex in `className` or inline `style`. It bypasses theming.

## Color as identity: the four-ramp ceiling

Some lookup tables answer "which thing is this?" with a color. Examples are a skill category, an event-log domain, a notification provider and a catalogue source. Stock Tailwind ramps keep creeping back into these tables, because a table with nine rows wants nine hues and the app ships four.

There are exactly four themeable ramps: `primary`, `sage`, `amber` and `coral`. Everything else in Tailwind's default palette (`emerald`, `violet`, `sky`, `teal`, `indigo`, `cyan`, `rose`, `pink`, `purple` and more) resolves to a fixed oklch value that ignores the active theme. A table built on those hues looks fine in the default skin and falls apart in every other one.

### The rule

1. Map a stock ramp to its themeable equivalent at the same shade step: `red` to `coral`, `green` and `emerald` to `sage`, `orange` to `amber`, `blue` to `primary`. `bg-emerald-50 text-emerald-700` becomes `bg-sage-50 text-sage-700`.

2. Hues with no equivalent do not get one. `violet`, `teal`, `sky`, `cyan`, `indigo`, `pink` and `purple` are not "nearly primary" or "nearly sage". Do not invent a fifth ramp, do not duplicate an existing one under a new name, and do not reach for `--accent-lavender` and similar. Those are fixed hexes, not ramps.

3. When a table needs more than four distinct hues, send the surplus rows to the neutral pair the table already defines (`bg-surface-subtle text-content-secondary`, or whatever its "unknown" or "other" row uses). Never let two rows share a ramp. Two domains that render identically destroy the distinction the table exists to show, which is worse than rendering one of them in neutral.

4. Decide which rows keep a hue by which distinction a reader acts on. The badge usually prints its own label, so color is a scanning aid, not the information. Spend the four ramps on the readings that change what someone does, and let the rest go neutral. Keep the meaning honest: `coral` reads as failure, so an ordinary row painted coral makes routine state look broken. Leaving a ramp unassigned is fine.

Worked examples in the tree:

| Table                                                     | Rows            | Kept a hue                                                                                  | Why                                                                                                                                           |
| --------------------------------------------------------- | --------------- | ------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------- |
| `skills/skillIcons.tsx` `CATEGORY_META`                   | 9               | `Built-in` (primary), `Productivity` (sage), `Social` (coral), `Tools & Automation` (amber) | `Channels`, `Chat` and `Platform` share the neutral tone of `All` / `Other`                                                                   |
| `skills/SkillsExplorerTab.tsx` `SOURCE_COLORS`            | 6               | `built-in` (sage), `optional` (primary)                                                     | The four remote catalogues print their own name; provenance tier is the distinction that matters                                              |
| `skills/SkillsExplorerTab.tsx` `FORMAT_MAP`               | 5 rows, 3 tones | Hermes family (primary), ClawHub family (sage), `legacy` (amber)                            | Three tones fit under the ceiling, so nothing is lost                                                                                         |
| `settings/panels/EventLogPanel.tsx` `DOMAIN_BADGE_COLORS` | 11              | `tool` (primary), `agent` (sage), `approval` (amber)                                        | Who acted, and what waits on a human. Coral stays unassigned, because no domain means failure                                                        |
| `notifications/NotificationCard.tsx` provider badge       | 6               | none                                                                                        | The importance badge in the same row already spends coral/amber/sage on high/medium/low; a coral provider would read as a failed notification |

### Brand tints are a separate question

A few plates use a third party's brand color, not an app hue: Telegram's `#249CD8`, Discord's `#5865F2` and iMessage's `#34C759` in `skills/skillIcons.tsx`. Flattening them to `bg-surface-subtle` would erase them into the generic badge beside them, so they stay as hex on purpose. Giving them a themeable home means adding brand tokens, which is a product decision, not a cleanup. The same goes for the provider badges in `NotificationCard.tsx`: reaching for a stock ramp is not the fix.

### Do not repaint a primitive's variant

`<Button variant="primary" className="bg-violet-500">` is the same bug. The variant already paints the accent ramp, and the override freezes the color and breaks the hover, focus and disabled states. Drop the override and retint the surface around the button instead.

## The migration codemod

`scripts/theme-codemod/` collapses audited light and `dark:` Tailwind pairings into the semantic utilities. It is idempotent and runs as a dry run by default:

```bash
node scripts/theme-codemod/migrate.mjs            # dry-run + report
node scripts/theme-codemod/migrate.mjs --write    # apply
node scripts/theme-codemod/migrate.mjs --selftest # fixture assertions
```

It only rewrites adjacent pairs and never touches opacity-suffixed utilities or test files. The mapping table is `scripts/theme-codemod/map.mjs`.
