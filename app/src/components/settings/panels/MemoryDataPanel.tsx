import { useCallback, useState } from 'react';

import type { ToastNotification } from '../../../types/intelligence';
import { MemoryWorkspace } from '../../intelligence/MemoryWorkspace';
import { ToastContainer } from '../../intelligence/Toast';
import MemoryWindowControl from '../components/MemoryWindowControl';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const MemoryDataPanel = () => {
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
      <SettingsHeader
        title="Memory Data"
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />
      <div className="p-4 space-y-4">
        <MemoryWindowControl onError={handleWindowError} onSaved={handleWindowSaved} />
        <MemoryWorkspace onToast={addToast} />
      </div>
      <ToastContainer notifications={toasts} onRemove={removeToast} />
    </div>
  );
};

export default MemoryDataPanel;
