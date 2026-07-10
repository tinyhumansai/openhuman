import { SettingsModalFrame } from './SettingsModalFrame';
import SettingsModalLayout from './SettingsModalLayout';
import { useCloseSettings } from './useCloseSettings';

/**
 * Desktop Settings, presented as a centered full-app-size modal floating over
 * the page it was opened from (the "background location"). Composes the
 * presentational {@link SettingsModalFrame} (backdrop / Esc / focus / close)
 * around the routed two-column {@link SettingsModalLayout}.
 *
 * Mounted by `AppShellDesktop` whenever the current path is a settings path; the
 * shell renders the background page underneath. iOS keeps the full-page
 * `pages/Settings.tsx` and never mounts this.
 */
export default function SettingsModal() {
  const close = useCloseSettings();
  return (
    <SettingsModalFrame onClose={close}>
      <SettingsModalLayout />
    </SettingsModalFrame>
  );
}
