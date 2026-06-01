import { useCallback, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import type { ToastNotification } from '../../../types/intelligence';
import { MemoryWorkspace } from '../../intelligence/MemoryWorkspace';
import { ToastContainer } from '../../intelligence/Toast';
import { VaultHealthChecklist } from '../../intelligence/VaultHealthChecklist';
import MemoryWindowControl from '../components/MemoryWindowControl';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

interface MemoryDataPanelProps {
  /** When true, render without the SettingsHeader chrome (used when embedded
   *  inside the onboarding custom wizard). */
  embedded?: boolean;
}

const MemoryDataPanel = ({ embedded = false }: MemoryDataPanelProps = {}) => {
  const { t } = useT();
  const { navigateBack, breadcrumbs } = useSettingsNavigation();
  const [toasts, setToasts] = useState<ToastNotification[]>([]);

  const addToast = useCallback((toast: Omit<ToastNotification, 'id'>) => {
    const newToast: ToastNotification = { ...toast, id: `toast-${Date.now()}-${Math.random()}` };
    setToasts(prev => [...prev, newToast]);
  }, []);

  const removeToast = (id: string) => {
    setToasts(prev => prev.filter(t => t.id !== id));
  };

  const handleWindowError = useCallback(
    (message: string) => {
      addToast({ type: 'error', title: 'Memory window', message });
    },
    [addToast]
  );

  const handleWindowSaved = useCallback(
    (window: string) => {
      addToast({ type: 'success', title: 'Memory window updated', message: `Set to ${window}.` });
    },
    [addToast]
  );

  return (
    <div className="z-10 relative">
      {!embedded && (
        <SettingsHeader
          title={t('memory.title')}
          showBackButton={true}
          onBack={navigateBack}
          breadcrumbs={breadcrumbs}
        />
      )}
      <div className={embedded ? 'space-y-4' : 'p-4 space-y-4'}>
        <section className="rounded-xl border border-stone-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 p-4 space-y-3">
          <h3 className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
            How memory storage works
          </h3>
          <dl className="space-y-2.5">
            <div>
              <dt className="text-xs font-semibold text-stone-900 dark:text-neutral-100">
                Workspace vault · write
              </dt>
              <dd className="text-xs leading-relaxed text-stone-600 dark:text-neutral-300">
                OpenHuman writes generated memory notes to
                <code className="mx-1 font-mono">memory_tree/content</code>.
              </dd>
            </div>
            <div>
              <dt className="text-xs font-semibold text-stone-900 dark:text-neutral-100">
                Connected sources · read
              </dt>
              <dd className="text-xs leading-relaxed text-stone-600 dark:text-neutral-300">
                Folders, mailboxes, chats, and repos are imported for memory indexing — their
                original files are never rewritten.
              </dd>
            </div>
            <div>
              <dt className="text-xs font-semibold text-stone-900 dark:text-neutral-100">
                Internal memory-tree files
              </dt>
              <dd className="text-xs leading-relaxed text-stone-600 dark:text-neutral-300">
                Indexes, queue state, and summaries are managed by OpenHuman to keep recall and sync
                healthy.
              </dd>
            </div>
          </dl>
        </section>
        <VaultHealthChecklist onToast={addToast} title="Vault setup health" />
        <MemoryWindowControl onError={handleWindowError} onSaved={handleWindowSaved} />
        <MemoryWorkspace onToast={addToast} />
      </div>
      <ToastContainer notifications={toasts} onRemove={removeToast} />
    </div>
  );
};

export default MemoryDataPanel;
