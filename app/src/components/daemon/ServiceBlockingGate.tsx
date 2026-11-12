import { useState } from 'react';

import { useDaemonHealth } from '../../hooks/useDaemonHealth';
import { useDaemonLifecycle } from '../../hooks/useDaemonLifecycle';
import { useT } from '../../lib/i18n/I18nContext';
import { useCoreState } from '../../providers/CoreStateProvider';
import { LATEST_APP_DOWNLOAD_URL } from '../../utils/config';
import { openUrl } from '../../utils/openUrl';
import { Button, ModalShell } from '../ui';

interface ServiceBlockingGateProps {
  children: React.ReactNode;
}

const ServiceBlockingGate = ({ children }: ServiceBlockingGateProps) => {
  const { t } = useT();
  const { snapshot } = useCoreState();
  const daemonHealth = useDaemonHealth();
  const daemonLifecycle = useDaemonLifecycle();
  const [isRestarting, setIsRestarting] = useState(false);
  const [restartError, setRestartError] = useState<string | null>(null);
  const hasSession = Boolean(snapshot.sessionToken);
  const shouldShowRecoveryPrompt =
    hasSession && daemonLifecycle.maxAttemptsReached && daemonHealth.status !== 'running';

  const handleRetry = async () => {
    setIsRestarting(true);
    setRestartError(null);
    try {
      const restarted = await daemonHealth.restartDaemon();
      if (restarted) {
        daemonLifecycle.resetRetries();
        return;
      }
      setRestartError(t('daemon.serviceBlockingGate.retryFailed'));
    } catch {
      setRestartError(t('daemon.serviceBlockingGate.retryFailed'));
    } finally {
      setIsRestarting(false);
    }
  };

  const handleDownloadLatest = async () => {
    await openUrl(LATEST_APP_DOWNLOAD_URL);
  };

  if (!shouldShowRecoveryPrompt) {
    return <>{children}</>;
  }

  return (
    <>
      {children}
      <ModalShell
        title={t('daemon.serviceBlockingGate.title')}
        titleId="daemon-recovery-title"
        describedBy="daemon-recovery-description"
        testId="daemon-recovery-dialog"
        closePolicy={{ escape: false, backdrop: false, button: false }}
        maxWidthClassName="max-w-xl"
        // Core recovery must remain operable above the mounted keyring and
        // announcement blockers (which reach z-10000).
        panelClassName="z-10002"
        overlayClassName="z-10001"
        footer={
          <div className="flex flex-wrap justify-end gap-3">
            <Button
              type="button"
              variant="secondary"
              size="sm"
              autoFocus
              onClick={() => void handleRetry()}
              disabled={isRestarting}>
              {isRestarting
                ? t('daemon.serviceBlockingGate.retrying')
                : t('daemon.serviceBlockingGate.retryCore')}
            </Button>
            <Button
              type="button"
              variant="primary"
              size="sm"
              onClick={() => void handleDownloadLatest()}>
              {t('daemon.serviceBlockingGate.downloadLatest')}
            </Button>
          </div>
        }>
        <div id="daemon-recovery-description" className="space-y-2 text-sm text-content-secondary">
          <p>{t('daemon.serviceBlockingGate.body')}</p>
          <p>{t('daemon.serviceBlockingGate.downloadHint')}</p>
          {restartError && (
            <p role="alert" className="text-coral-500">
              {restartError}
            </p>
          )}
        </div>
      </ModalShell>
    </>
  );
};

export default ServiceBlockingGate;
