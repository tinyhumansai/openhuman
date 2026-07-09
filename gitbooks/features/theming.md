---
description: >-
  Five built-in theme families, light/dark/auto variants, and a full visual
  Theme Studio - make OpenHuman look the way you want.
icon: palette
---

# Themes & Theme Studio

OpenHuman is fully re-skinnable at runtime. Pick from built-in themes, switch light/dark/auto, or open the **Theme Studio** to design your own. Every change applies instantly and persists locally, no restart required.

***

## Built-in themes

Five theme families ship out of the box, each with light and dark variants:

| Family       | Feel                                  |
| ------------ | ------------------------------------- |
| **Classic**  | The default OpenHuman look.           |
| **Ocean**    | Cool blues around the `#4A83DD` primary. |
| **Sepia**    | Warm, paper-like, easy on the eyes.   |
| **Matrix**   | High-contrast green-on-black.         |
| **HAL 9000** | Deep black with a red accent.         |

Each can be applied as **Light**, **Dark**, or **Auto**. Auto follows your operating system's `prefers-color-scheme` setting and re-applies live the moment you flip your OS between light and dark, with no reload.

***

## Theme Studio

**Settings → Theme Studio** is a full visual editor. From it you can:

* **Pick a family** from a gallery of theme tiles (built-ins plus your own custom themes).
* **Adjust every colour token** with colour pickers: surfaces, text, borders, and accent ramps. A live **contrast warning** flags when text-on-background luminance drops below a readable threshold.
* **Swap fonts per role**: title, heading, body, mono, and serif can each use a different family.
* **Configure the backdrop**: an animated WebGL mesh, a flat colour, or a custom image, with an optional dotted overlay.
* **Manage custom themes**: create, edit, reset, delete, and **export / import as JSON** to share a theme with someone else.

### Editing a preset auto-forks

Changing any token on a built-in preset transparently creates a **new custom theme**. The original preset stays pristine. So you can start from Ocean, tweak it, and keep both.

***

## Where it's stored

Theme state (the active theme, the light/dark/auto variant, and all your custom themes) lives in Redux and persists to `localStorage` via `redux-persist`, so it survives app restarts and is scoped to your user. Sharing a theme is just exporting the JSON and having someone import it.

***

## Under the hood

Themes are driven by CSS custom-property **tokens** (space-separated RGB channel triples, so Tailwind opacity modifiers like `bg-surface/50` keep working). The `ThemeProvider` writes the active theme's overrides onto the `<html>` root; unspecified tokens fall through to the light/dark defaults.

For the full token taxonomy, Tailwind wiring, and component-authoring best practices, see the contributor reference: [Theming (developing)](../developing/theming.md).

***

## See also

* [Theming (contributor reference)](../developing/theming.md): token system, Tailwind wiring, migration codemod.
* [Realtime Mascot](mascot/README.md): the other big piece of OpenHuman's personality.
