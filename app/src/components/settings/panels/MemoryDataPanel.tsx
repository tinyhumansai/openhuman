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
        <section className="rounded-xl border border-stone-200 dark:border-neutral-700 bg-stone-50 dark:bg-neutral-900/60 p-4 space-y-2">
          <h2 className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
            Memory Read/Write Model
          </h2>
          <p className="text-xs leading-relaxed text-stone-700 dark:text-neutral-300">
            OpenHuman writes generated notes to the workspace vault at
            <code className="mx-1 font-mono">memory_tree/content</code>. Connected sources you add
            (Gmail, Slack, folders, repos) are read and imported into memory; OpenHuman does not
            rewrite those original sources.
          </p>
          <p className="text-xs leading-relaxed text-stone-700 dark:text-neutral-300">
            Internal memory-tree files (indexes, queue state, summaries) are managed by OpenHuman
            for retrieval and sync health.
          </p>
        </section>
        <VaultHealthChecklist onToast={addToast} title="Vault Setup Health" />
        <MemoryWindowControl onError={handleWindowError} onSaved={handleWindowSaved} />
        <MemoryWorkspace onToast={addToast} />
      </div>
      <ToastContainer notifications={toasts} onRemove={removeToast} />
    </div>
  );
};

export default MemoryDataPanel;
