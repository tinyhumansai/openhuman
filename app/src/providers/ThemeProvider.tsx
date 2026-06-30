import { ReactNode, useCallback, useEffect, useRef } from 'react';

import { findFamily, resolveFamilyVariant } from '../lib/theme/presets';
import type { Theme } from '../lib/theme/types';
import { useAppSelector } from '../store/hooks';
import {
  FONT_SIZE_PX,
  selectActiveFamilyId,
  selectEffectiveTheme,
  selectThemeVariant,
} from '../store/themeSlice';

/**
 * Applies the active {@link Theme} to the root `<html>` element:
 *
 * - Writes each colour token override as an inline `--<key>` CSS variable and
 *   each font role as `--font-<role>`. Anything the theme omits falls through to
 *   the Light/Dark defaults in `styles/tokens.css`. Variables set by a previous
 *   theme but absent from the new one are removed, so switching themes never
 *   leaks stale overrides.
 * - Toggles the `.dark` class from `theme.isDark` so the tokens.css dark base
 *   and any remaining `dark:` utilities activate together.
 * - `theme.mode === system` (active id `system`) also subscribes to
 *   `prefers-color-scheme` so OS-level flips re-apply live without a reload.
 * - `theme.fontSize` → the `<html>` inline `font-size` (rem-based Tailwind text
 *   utilities scale off this), unchanged from before.
 */
const ThemeProvider = ({ children }: { children: ReactNode }) => {
  const fontSize = useAppSelector(state => state.theme.fontSize);
  const themeVariant = useAppSelector(selectThemeVariant);
  const activeFamilyId = useAppSelector(selectActiveFamilyId);
  const effectiveTheme = useAppSelector(selectEffectiveTheme);

  // Track which inline vars we set last time so we can clear stale ones.
  const appliedRef = useRef<{ colors: string[]; fonts: string[] }>({ colors: [], fonts: [] });

  const applyTheme = useCallback((theme: Theme) => {
    if (typeof document === 'undefined') return;
    const root = document.documentElement;

    if (theme.isDark) root.classList.add('dark');
    else root.classList.remove('dark');
    root.style.colorScheme = theme.isDark ? 'dark' : 'light';

    const prev = appliedRef.current;
    for (const key of prev.colors) {
      if (!(key in theme.colors)) root.style.removeProperty(`--${key}`);
    }
    for (const role of prev.fonts) {
      if (!theme.fonts[role as keyof typeof theme.fonts]) {
        root.style.removeProperty(`--font-${role}`);
      }
    }

    for (const [key, value] of Object.entries(theme.colors)) {
      root.style.setProperty(`--${key}`, value);
    }
    for (const [role, stack] of Object.entries(theme.fonts)) {
      if (stack) root.style.setProperty(`--font-${role}`, stack);
    }

    // Optional decorative canvas gradient (behind all surfaces).
    if (theme.gradient?.canvas) {
      root.style.setProperty('--app-gradient', theme.gradient.canvas);
    } else {
      root.style.removeProperty('--app-gradient');
    }

    appliedRef.current = { colors: Object.keys(theme.colors), fonts: Object.keys(theme.fonts) };
    console.debug('[theme] applied', {
      id: theme.id,
      isDark: theme.isDark,
      colorOverrides: Object.keys(theme.colors).length,
      fontOverrides: Object.keys(theme.fonts).length,
    });
  }, []);

  // Apply the global font size to <html>.
  useEffect(() => {
    if (typeof document === 'undefined') return;
    const px = FONT_SIZE_PX[fontSize] ?? FONT_SIZE_PX.medium;
    console.debug('[theme] applying root font-size', { fontSize, px });
    document.documentElement.style.fontSize = px;
  }, [fontSize]);

  // Apply the active theme whenever it changes.
  useEffect(() => {
    applyTheme(effectiveTheme);
  }, [effectiveTheme, applyTheme]);

  // When the active family follows the OS preference (Auto variant), re-apply on
  // system light/dark flips. Resolves the *active family's* variant (not just
  // the classic preset), and stays inert for explicit light/dark or a custom
  // theme (selectActiveFamilyId is '' for custom selections).
  useEffect(() => {
    if (themeVariant !== 'system' || !activeFamilyId) return;
    if (typeof window === 'undefined' || !window.matchMedia) return;
    const mq = window.matchMedia('(prefers-color-scheme: dark)');
    const listener = () => {
      const family = findFamily(activeFamilyId) ?? findFamily('classic');
      if (family) applyTheme(resolveFamilyVariant(family, mq.matches ? 'dark' : 'light'));
    };
    if (mq.addEventListener) {
      mq.addEventListener('change', listener);
      return () => mq.removeEventListener('change', listener);
    }
    // Safari < 14 fallback.
    mq.addListener(listener);
    return () => mq.removeListener(listener);
  }, [themeVariant, activeFamilyId, applyTheme]);

  return <>{children}</>;
};

export default ThemeProvider;
