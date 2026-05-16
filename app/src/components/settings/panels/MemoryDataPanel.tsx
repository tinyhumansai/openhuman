import { useCallback, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import type { ToastNotification } from '../../../types/intelligence';
import { MemoryWorkspace } from '../../intelligence/MemoryWorkspace';
import { ToastContainer } from '../../intelligence/Toast';
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
        <MemoryWindowControl onError={handleWindowError} onSaved={handleWindowSaved} />
        <MemoryWorkspace onToast={addToast} />
      </div>
      <ToastContainer notifications={toasts} onRemove={removeToast} />
    </div>
  );
};

export default MemoryDataPanel;
