/**
 * useDeveloperMode — runtime developer-mode gate.
 *
 * Returns `true` when developer surfaces should be visible.  The gate is open
 * when EITHER the build is a Vite dev build (`IS_DEV`) OR the user has
 * enabled Developer Mode in Settings › About.
 *
 * Gating is UI-only.  The Rust `SecurityPolicy` / autonomy-tier enforcement
 * in the core is authoritative and is never relaxed by this toggle.
 */
import { useAppSelector } from '../store/hooks';
import { selectDeveloperMode } from '../store/themeSlice';
import { IS_DEV } from '../utils/config';

export function useDeveloperMode(): boolean {
  const persistedMode = useAppSelector(selectDeveloperMode);
  return IS_DEV || persistedMode;
}
