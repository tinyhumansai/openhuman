import { useT } from '../../lib/i18n/I18nContext';
import Kbd from '../commands/Kbd';
import { ModalShell } from '../ui/ModalShell';
import { ShortcutsList } from './shortcutsView';

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

/**
 * The keyboard-shortcuts help directory. Opened with `?` or ⌘/Ctrl+/ (see
 * `globalActions`), it lists every currently-active shortcut, grouped, straight
 * from the command registry so it can't drift from the real bindings.
 */
export default function KeyboardShortcutsModal({ open, onOpenChange }: Props) {
  const { t } = useT();

  if (!open) return null;

  return (
    <ModalShell
      onClose={() => onOpenChange(false)}
      title={t('shortcuts.title')}
      titleId="keyboard-shortcuts-title"
      subtitle={t('shortcuts.subtitle')}
      maxWidthClassName="max-w-lg"
      contentClassName="px-5 py-4">
      <div className="max-h-[60vh] overflow-y-auto pr-1" data-testid="keyboard-shortcuts-list">
        <ShortcutsList variant="modal" />
      </div>
      <p className="mt-4 flex items-center justify-center gap-1.5 text-xs text-stone-400 dark:text-neutral-500">
        {t('shortcuts.openHint')}
        <Kbd shortcut="?" />
      </p>
    </ModalShell>
  );
}
