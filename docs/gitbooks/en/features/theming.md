---
description: >-
  Five theme families with light, dark and auto variants, a Theme Studio for
  colors, fonts and backdrops, and Appearance controls for text size, corner
  rounding and borders.
icon: palette
---

# Themes and Theme Studio

You can re-skin OpenHuman at runtime. Every change applies instantly, is saved locally and needs no restart.

The controls sit in two places:

| Page | What you set there |
| --- | --- |
| Settings → Appearance (`/settings/appearance`) | Text size, corner rounding, border contrast, per-area borders |
| Settings → Theme Studio (`/settings/theme`) | Theme family, light, dark or auto, every color token, fonts per role, the backdrop and custom-theme management |

## Built-in themes

Five families ship in `app/src/lib/theme/presets.ts`, each with a light and a dark variant:

| Family | Feel | Default variant |
| --- | --- | --- |
| Classic | The default OpenHuman look. | Light |
| Ocean | Cool blues. | Light |
| Sepia | Warm and paper-like, set in a serif. | Light |
| Matrix | High-contrast green on black, set in a monospace. | Dark |
| HAL 9000 | Deep black with a red accent. | Dark |

Each family is applied as Light, Dark or Auto. Auto follows your operating system's `prefers-color-scheme` and re-applies the moment you switch the OS between light and dark, with no reload.

Classic has no color or font overrides of its own on purpose. It is the palette in `app/src/styles/tokens.css`, chosen by the light and dark switch alone.

## Colors

The Theme Studio exposes every color token, grouped as Surfaces, Text, Borders and Accent colors. The four accent ramps (primary, sage, amber, coral) show their `500` shade by default and expand to all eleven shades from `50` to `950`.

Each token is stored as a space-separated RGB channel triple such as `255 255 255`, not a hex string. Tailwind wraps every token as `rgb(var(--token) / <alpha-value>)`, so opacity modifiers like `bg-surface/50` keep working. The reasoning is at the top of `app/src/styles/tokens.css`.

A contrast warning appears when the luminance gap between `content` and `surface-canvas` falls under 0.2. It is advisory and does not block the change.

## Fonts

Each of five roles can take a different font family:

| Role | Used for |
| --- | --- |
| `title` | Display and brand type |
| `heading` | Section headings |
| `body` | Everything else |
| `mono` | Code and fixed-width output |
| `serif` | Serif passages |

All five pick from the same six choices in `app/src/lib/theme/tokens.ts` (`FONT_CHOICES`): Inter, Cabinet Grotesk, System UI, Newsreader (serif), Georgia (serif) and JetBrains Mono. A role whose stored stack matches no choice shows a disabled Current entry instead of silently switching to another font.

Cabinet Grotesk is not bundled, so every stack naming it resolves to Inter. The name stays so the intended stack is still readable. See the note in `app/src/styles/tokens.css`.

## Backdrop

Three kinds, set in the Theme Studio's Background card:

| Kind | What it paints |
| --- | --- |
| `solid` | Nothing of its own: the themed body colour shows through. The default. |
| `mesh` | An animated, theme-tinted WebGL mesh gradient. |
| `image` | A cover image from a URL you supply. |

No built-in preset sets a backdrop, so every family starts on `solid` and the animated shader is opt-in.

## Text size

Four tiers, applied as an inline `font-size` on the root `<html>` element so everything rem-based scales with it:

| Tier | Size |
| --- | --- |
| Small | 14px |
| Medium | 16px (default) |
| Large | 18px |
| Extra large | 20px |

A Custom size number field and slider take any whole pixel value. `clampFontSizePx` in `app/src/store/themeSlice.ts` clamps it to 12px to 28px, so you can go denser than Small or larger than Extra large. A non-finite value falls back to 16. Picking a tier clears the custom value.

## Layout

Three controls in `app/src/lib/theme/layout.ts`, all under Appearance, with a Reset layout button that restores every default.

Corner rounding scales Tailwind's whole radius scale (`--radius-xs` through `--radius-5xl`) by one factor:

| Option | Factor |
| --- | --- |
| None | 0 |
| Subtle | 0.5 |
| Default | 1 (the variables are removed, so the stylesheet wins) |
| Round | 1.6 |

Pills and avatars use `rounded-full` rather than a radius token, so they stay round at any setting.

Border contrast adjusts `--line`, `--line-strong` and `--line-subtle`, and leaves `--line-chrome` to the theme. The tokens reset before each change, so levels never compound:

| Option | Effect |
| --- | --- |
| Subtle | Mixes the line tokens 55 percent toward the surface |
| Default | No change. The theme's own values stand |
| Strong | Mixes the line tokens 30 percent toward the text colour |

Per-area borders turn borders off for one area at a time. They set a `data-borders-<area>="off"` attribute that `app/src/styles/layout-overrides.css` acts on. There are four areas:

| Area | Covers |
| --- | --- |
| Cards and panels | Settings cards, grouped sections and panels |
| Form controls | Text fields, text boxes and dropdowns |
| Row dividers | Hairlines between rows in lists |
| Window frame | The edge around the main content area |

## Custom themes

Changing any color, font or backdrop on a built-in preset forks it into a new custom theme named `<Name> (custom)` and makes that active. The preset stays untouched, so you can start from Ocean, tweak it and keep both. Forking happens once per source theme.

A custom theme also offers Reset overrides (back to the preset it was forked from, or empty if it was not forked) and Delete theme, which falls back to Classic.

You share themes as JSON on the clipboard, not as a file. Copy JSON puts the whole theme object on your clipboard and also shows it in a read-only text box to copy by hand. Import is a paste field. The JSON needs a `colors` object whose values are all strings (an empty one is accepted). An unrecognized backdrop kind is dropped instead of failing the import. An imported theme gets a fresh id, so importing never overwrites what you have.

## How a theme is applied

A theme is a set of values for the tokens in `app/src/styles/tokens.css`. `applyTheme` in `app/src/providers/ThemeProvider.tsx` writes the active theme's colors and fonts as inline `--token` custom properties on the root `<html>` element. They override the stylesheet's light and dark blocks at runtime.

Presets and fully custom themes use the same mechanism. There is no per-theme stylesheet class and no `data-theme` attribute. The only class involved is `.dark`, a light and dark base selector that also keeps Tailwind `dark:` utilities aligned. Any token a theme leaves out falls through to the stylesheet defaults. Tokens set by a previous theme are cleared on each switch, so nothing leaks.

Theme state persists through `redux-persist` under the `theme` key in plain `localStorage`, not `userScopedStorage`. The theme is a pre-login, whole-app preference, so it survives switching users instead of being scoped to one.

## Not wired

Two fields in `app/src/store/themeSlice.ts` are persisted but do nothing. Neither has a UI control or a consumer:

| Field | State |
| --- | --- |
| `developerMode` | Has a reducer and a `selectDeveloperMode` selector, but nothing dispatches or reads either. Its doc comment points at a `useDeveloperMode` hook that does not exist. Developer Options gates on core mode instead. |
| `tabBarLabels` | Has a reducer and no selector, so nothing can read it. |

Both are in the persist whitelist, so a stale default may sit in `localStorage`. Editing either by hand has no visible effect.

## See also

- [Theming (contributor reference)](../developing/theming.md): the token taxonomy, Tailwind wiring and component-authoring rules.
- [The mascot](mascot/README.md): the other large piece of OpenHuman's personality.
- [Platform and availability](platform.md): which desktop platforms the shell ships on.
