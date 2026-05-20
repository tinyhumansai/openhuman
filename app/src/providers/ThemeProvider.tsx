import { ReactNode, useEffect } from 'react';

import { useAppSelector } from '../store/hooks';
import { resolveTheme, type ThemeMode } from '../store/themeSlice';

/**
 * Syncs the Redux `theme.mode` slice to the `<html>` element's class list so
 * Tailwind's `darkMode: 'class'` and the `:root.dark` CSS variable block in
 * theme.css activate together.
 *
 * Mode = `system` also subscribes to `prefers-color-scheme` so OS-level theme
 * flips propagate live without a reload.
 */
const ThemeProvider = ({ children }: { children: ReactNode }) => {
  const mode = useAppSelector(state => state.theme.mode) as ThemeMode;

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
