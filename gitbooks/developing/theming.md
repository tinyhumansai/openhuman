# Theming

OpenHuman is fully re-skinnable at runtime. Colours and fonts are driven by CSS
variables (the "tokens"), so a theme is just a set of values for those variables.
This page is the contributor reference for the token system.

## How it works

1. **Tokens** — `app/src/styles/tokens.css` defines every themeable colour as a
   space-separated **RGB channel triple** (e.g. `--surface: 255 255 255;`) plus
   font-role vars (`--font-title/heading/body/mono/serif`). The Light palette
   lives in `:root`; the Dark palette in `:root.dark`.

2. **Tailwind wiring** — `app/tailwind.config.js` exposes the tokens as utility
   colours via `rgb(var(--token) / <alpha-value>)`. The `<alpha-value>` form is
   what keeps opacity modifiers working (`bg-surface/50`, `bg-primary-500/10`).
   Channel format is mandatory for this reason — never store a token as a hex
   string.

3. **Runtime application** — `app/src/providers/ThemeProvider.tsx` resolves the
   active `Theme` and writes its overrides as inline `--token` / `--font-<role>`
   variables on `<html>`, toggling `.dark` from `theme.isDark`. Variables a theme
   doesn't override fall through to the tokens.css defaults; variables left over
   from a previous theme are removed on switch.

4. **State** — `app/src/store/themeSlice.ts` holds `activeThemeId` and
   `customThemes`. Built-in presets live in `app/src/lib/theme/presets.ts`.
   Users edit themes in **Settings → Theme Studio**
   (`app/src/components/settings/panels/ThemeStudioPanel.tsx`).

## Token taxonomy

| Group     | Tokens                                                                                  | Tailwind utilities |
| --------- | --------------------------------------------------------------------------------------- | ------------------ |
| Surfaces  | `surface`, `surface-canvas`, `surface-muted`, `surface-subtle`, `surface-strong`, `surface-hover`, `surface-overlay` | `bg-surface`, `bg-surface-muted`, … |
| Text      | `content`, `content-secondary`, `content-muted`, `content-faint`, `content-inverted`    | `text-content`, `text-content-muted`, … |
| Borders   | `line`, `line-strong`, `line-subtle`                                                     | `border-line`, `border-line-strong`, … |
| Accents   | `primary-*`, `sage-*`, `amber-*`, `coral-*` (shades 50…950)                              | `bg-primary-500`, `text-coral-600`, … (var-backed, themeable, unchanged names) |
| Fonts     | `font-title`, `font-heading`, `font-body`, `font-mono`, `font-serif`                     | `font-title`, `font-heading`, `font-body`, … |

The legacy `--cmd-*` and `--color-*` variable sets are thin aliases over these
canonical tokens — don't add new colours there.

## Authoring components

- Use semantic utilities (`bg-surface`, `text-content`, `border-line`) for
  neutral surfaces/text/borders instead of `bg-white dark:bg-neutral-900` etc.
  You almost never need `dark:` variants for these — the token flips for you.
- Use the accent palettes (`primary`/`sage`/`amber`/`coral`) for semantic colour;
  they're themeable with no extra work.
- Avoid hardcoded hex in `className` or inline `style` — those bypass theming.

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
