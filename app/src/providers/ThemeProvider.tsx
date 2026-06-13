import { ReactNode, useEffect } from 'react';

import { useAppSelector } from '../store/hooks';
import { FONT_SIZE_PX, resolveTheme, type ThemeMode } from '../store/themeSlice';

/**
 * Syncs the Redux `theme` slice to the root `<html>` element:
 *
 * - `theme.mode` → the `<html>` class list so Tailwind's `darkMode: 'class'`
 *   and the `:root.dark` CSS variable block in theme.css activate together.
 *   Mode = `system` also subscribes to `prefers-color-scheme` so OS-level theme
 *   flips propagate live without a reload.
 * - `theme.fontSize` (issue #3120) → the `<html>` inline `font-size`. Because
 *   Tailwind's text scale is rem-based and `:root` is 16px, scaling the root
 *   font-size scales every text utility app-wide (chat messages, composer, UI
 *   chrome) independently of the OS / system font setting.
 */
const ThemeProvider = ({ children }: { children: ReactNode }) => {
  const mode = useAppSelector(state => state.theme.mode) as ThemeMode;
  const fontSize = useAppSelector(state => state.theme.fontSize);

  // Apply the global font size to <html>. rem-based Tailwind utilities scale
  // off this, so a single inline style flows through the whole tree.
  useEffect(() => {
    if (typeof document === 'undefined') return;
    const px = FONT_SIZE_PX[fontSize] ?? FONT_SIZE_PX.medium;
    console.debug('[theme] applying root font-size', { fontSize, px });
    document.documentElement.style.fontSize = px;
  }, [fontSize]);

  useEffect(() => {
    const apply = () => {
      const root = document.documentElement;
      const resolved = resolveTheme(mode);
      if (resolved === 'dark') {
        root.classList.add('dark');
      } else {
        root.classList.remove('dark');
      }
      root.style.colorScheme = resolved;
    };

    apply();

    if (mode !== 'system') return;
    if (typeof window === 'undefined' || !window.matchMedia) return;
    const mq = window.matchMedia('(prefers-color-scheme: dark)');
    const listener = () => apply();
    // Safari < 14 uses addListener/removeListener (the deprecated API). Guard
    // for both so we don't ship a broken sync on older webviews.
    if (mq.addEventListener) {
      mq.addEventListener('change', listener);
      return () => mq.removeEventListener('change', listener);
    }
    mq.addListener(listener);
    return () => mq.removeListener(listener);
  }, [mode]);

  return <>{children}</>;
};

export default ThemeProvider;
