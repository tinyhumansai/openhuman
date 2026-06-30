import { useState } from 'react';

import { useDaemonHealth } from '../../hooks/useDaemonHealth';
import { useDaemonLifecycle } from '../../hooks/useDaemonLifecycle';
import { useT } from '../../lib/i18n/I18nContext';
import { useCoreState } from '../../providers/CoreStateProvider';
import { LATEST_APP_DOWNLOAD_URL } from '../../utils/config';
import { openUrl } from '../../utils/openUrl';

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
      <div className="fixed inset-0 z-[10000] bg-stone-950/80 backdrop-blur-sm flex items-center justify-center p-4">
        <div className="w-full max-w-xl rounded-2xl border border-coral-500/30 bg-stone-900 p-6 shadow-2xl">
          <h2 className="text-xl font-semibold text-white">
            {t('daemon.serviceBlockingGate.title')}
          </h2>
          <p className="mt-2 text-sm text-content-faint">{t('daemon.serviceBlockingGate.body')}</p>
          <p className="mt-2 text-sm text-content-faint">
            {t('daemon.serviceBlockingGate.downloadHint')}
          </p>
          {restartError && <p className="mt-3 text-sm text-coral-300">{restartError}</p>}
          <div className="mt-5 flex gap-3">
            <button
              type="button"
              onClick={handleRetry}
              disabled={isRestarting}
              className="rounded-lg border border-stone-600 px-4 py-2 text-sm text-stone-100 hover:bg-stone-800 disabled:opacity-60">
              {isRestarting
                ? t('daemon.serviceBlockingGate.retrying')
                : t('daemon.serviceBlockingGate.retryCore')}
            </button>
            <button
              type="button"
              onClick={handleDownloadLatest}
              className="rounded-lg bg-coral-500 px-4 py-2 text-sm font-medium text-content-inverted hover:bg-coral-600">
              {t('daemon.serviceBlockingGate.downloadLatest')}
            </button>
          </div>
        </div>
      </div>
    </>
  );
};

export default ServiceBlockingGate;
