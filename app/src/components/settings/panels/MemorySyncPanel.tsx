import { useCallback, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import type { ToastNotification } from '../../../types/intelligence';
import { MemorySourcesRegistry } from '../../intelligence/MemorySourcesRegistry';
import { SyncAuditPanel } from '../../intelligence/SyncAuditPanel';
import { ToastContainer } from '../../intelligence/Toast';
import SettingsPanel from '../layout/SettingsPanel';

/**
 * Data Sync — top-level Settings → Account page (#3301).
 *
 * The single, focused home for "what syncs, how much, how fresh, and is it
 * running right now". Shows the source registry (per-source rows, opt-in
 * toggles, live "syncing now" indicator, per-source settings, and the global
 * sync-schedule control) plus the Sync History audit table.
 *
 * The "Memory Tree" status tiles / per-integration health and the
 * force-directed memory graph deliberately stay on the developer Memory page
 * (Dev & Diagnostics → Memory), not here.
 */
const MemorySyncPanel = () => {
  const { t } = useT();
  const [toasts, setToasts] = useState<ToastNotification[]>([]);

  const addToast = useCallback((toast: Omit<ToastNotification, 'id'>) => {
    const newToast: ToastNotification = { ...toast, id: `toast-${Date.now()}-${Math.random()}` };
    setToasts(prev => [...prev, newToast]);
  }, []);

  const removeToast = (id: string) => {
    setToasts(prev => prev.filter(toast => toast.id !== id));
  };

  return (
    <SettingsPanel description={t('settings.dataSync.menuDesc')}>
      <p className="text-sm text-content-muted">{t('settings.dataSync.description')}</p>
      <MemorySourcesRegistry onToast={addToast} />
      <div className="rounded-lg border border-line bg-surface p-4">
        <h3 className="mb-2 text-sm font-semibold text-content">
          {t('sync.auditTitle', 'Sync History')}
        </h3>
        <SyncAuditPanel />
      </div>
      <ToastContainer notifications={toasts} onRemove={removeToast} />
    </SettingsPanel>
  );
};

export default MemorySyncPanel;
