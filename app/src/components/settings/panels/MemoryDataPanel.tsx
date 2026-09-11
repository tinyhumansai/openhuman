import { useCallback, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import type { ToastNotification } from '../../../types/intelligence';
import { MemoryWorkspace } from '../../intelligence/MemoryWorkspace';
import { ToastContainer } from '../../intelligence/Toast';
import { VaultHealthChecklist } from '../../intelligence/VaultHealthChecklist';
import PanelPage from '../../layout/PanelPage';
import MemoryWindowControl from '../components/MemoryWindowControl';
import { SettingsSection } from '../controls';
import SettingsPanel from '../layout/SettingsPanel';

interface MemoryDataPanelProps {
  /** When true, render without the SettingsHeader chrome (used when embedded
   *  inside the onboarding custom wizard). */
  embedded?: boolean;
}

const MemoryDataPanel = ({ embedded = false }: MemoryDataPanelProps = {}) => {
  const { t } = useT();
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
      addToast({ type: 'error', title: t('memoryData.windowError'), message });
    },
    [addToast, t]
  );

  const handleWindowSaved = useCallback(
    (window: string) => {
      addToast({
        type: 'success',
        title: t('memoryData.windowUpdated'),
        message: t('memoryData.windowUpdatedMsg').replace('{window}', window),
      });
    },
    [addToast, t]
  );

  const body = (
    <>
      <div className="space-y-5">
        <SettingsSection title={t('memoryData.howItWorks')}>
          <dl className="space-y-2.5 px-4 py-3">
            <div>
              <dt className="text-xs font-semibold text-content">
                {t('memoryData.workspaceVault')}
              </dt>
              <dd className="text-xs leading-relaxed text-content-muted">
                {t('memoryData.workspaceVaultDesc')}
              </dd>
            </div>
            <div>
              <dt className="text-xs font-semibold text-content">
                {t('memoryData.connectedSources')}
              </dt>
              <dd className="text-xs leading-relaxed text-content-muted">
                {t('memoryData.connectedSourcesDesc')}
              </dd>
            </div>
            <div>
              <dt className="text-xs font-semibold text-content">
                {t('memoryData.internalFiles')}
              </dt>
              <dd className="text-xs leading-relaxed text-content-muted">
                {t('memoryData.internalFilesDesc')}
              </dd>
            </div>
          </dl>
        </SettingsSection>
        <VaultHealthChecklist onToast={addToast} title={t('vaultHealth.setupTitle')} />
        <MemoryWindowControl onError={handleWindowError} onSaved={handleWindowSaved} />
        <MemoryWorkspace onToast={addToast} />
      </div>
      <ToastContainer notifications={toasts} onRemove={removeToast} />
    </>
  );

  // Embedded (the onboarding custom wizard) keeps the headerless PanelPage
  // branch — that host draws its own step chrome, so a settings page header
  // here would be a second one.
  if (embedded) {
    return (
      <PanelPage className="z-10" contentClassName="">
        {body}
      </PanelPage>
    );
  }

  return <SettingsPanel description={t('devOptions.memoryInspectionDesc')}>{body}</SettingsPanel>;
};

export default MemoryDataPanel;
