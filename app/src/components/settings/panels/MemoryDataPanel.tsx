import { useState } from 'react';

import type { ToastNotification } from '../../../types/intelligence';
import { MemoryWorkspace } from '../../intelligence/MemoryWorkspace';
import { ToastContainer } from '../../intelligence/Toast';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const MemoryDataPanel = () => {
  const { navigateBack } = useSettingsNavigation();
  const [toasts, setToasts] = useState<ToastNotification[]>([]);

  const addToast = (toast: Omit<ToastNotification, 'id'>) => {
    const newToast: ToastNotification = { ...toast, id: `toast-${Date.now()}-${Math.random()}` };
    setToasts(prev => [...prev, newToast]);
  };

  const removeToast = (id: string) => {
    setToasts(prev => prev.filter(t => t.id !== id));
  };

  return (
    <div className="z-10 relative">
      <SettingsHeader title="Memory Data" showBackButton={true} onBack={navigateBack} />
      <div className="p-4">
        <MemoryWorkspace onToast={addToast} />
      </div>
      <ToastContainer notifications={toasts} onRemove={removeToast} />
    </div>
  );
};

export default MemoryDataPanel;
