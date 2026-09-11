/**
 * `cn()` — the single class-name composer for the UI primitive layer.
 *
 * Two jobs: `clsx` flattens conditionals, `tailwind-merge` resolves conflicts
 * last-wins so a caller's `className` actually beats a component's default
 * instead of emitting two competing utilities.
 *
 * WHY THE CONFIG BELOW EXISTS: tailwind-merge does not read
 * `tailwind.config.js`. It ships a model of the *stock* Tailwind scales, and
 * this repo overrides several of them. A custom key it does not know about is
 * not treated as "unknown and harmless" — for three groups it is silently
 * classified into a *different* group (colour), and then dropped by a later
 * class in that group. `cn('text-micro', 'text-content')` would lose the font
 * size. `cn.test.ts` reads `tailwind.config.js` and fails if the two drift.
 *
 * Use `extend`, never `override`: `override` would replace a whole group and
 * drop the stock keys (`text-sm`, `rounded-lg`, `shadow-md`) that the existing
 * ~640 opacity-suffixed utilities across the app depend on.
 */
import { type ClassValue, clsx } from 'clsx';
import { extendTailwindMerge } from 'tailwind-merge';

const twMerge = extendTailwindMerge({
  extend: {
    classGroups: {
      // `text-micro` would otherwise be read as a TEXT COLOUR.
      'font-size': [{ text: ['micro'] }],
      // Stock Tailwind has no `rounded-xs` / `rounded-5xl`.
      rounded: [{ rounded: ['xs', '4xl', '5xl'] }],
      // These would otherwise be read as SHADOW COLOURS and never conflict
      // with `shadow-xl`.
      shadow: [
        {
          shadow: [
            'glow',
            'glow-lg',
            'inner-glow',
            'subtle',
            'soft',
            'medium',
            'large',
            'float',
            'crisp',
            'cmd-palette',
            'content-edge',
          ],
        },
      ],
      // These would otherwise be read as BACKGROUND COLOURS.
      'bg-image': [{ bg: ['noise', 'gradient-mesh', 'gradient-radial', 'gradient-conic'] }],
      'backdrop-blur': [{ 'backdrop-blur': ['xs', '2xl', '3xl'] }],
    },
  },
});

export function cn(...inputs: ClassValue[]): string {
  return twMerge(clsx(inputs));
}

export default cn;
