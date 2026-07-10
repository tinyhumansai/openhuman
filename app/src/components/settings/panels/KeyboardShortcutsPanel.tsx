/**
 * Settings → Keyboard Shortcuts.
 *
 * A discoverable home for the app-wide shortcut map. Renders the same live,
 * registry-driven list as the `?` help overlay (see `shortcutsView`), so the
 * two surfaces can never disagree.
 */
import { useT } from '../../../lib/i18n/I18nContext';
import { ShortcutsList } from '../../shortcuts/shortcutsView';
import SettingsPanel from '../layout/SettingsPanel';

const KeyboardShortcutsPanel = () => {
  const { t } = useT();
  return (
    <SettingsPanel description={t('shortcuts.subtitle')} testId="settings-keyboard-shortcuts">
      <ShortcutsList variant="panel" />
    </SettingsPanel>
  );
};

export default KeyboardShortcutsPanel;
