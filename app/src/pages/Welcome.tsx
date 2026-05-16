import createDebug from 'debug';
import { useState } from 'react';

import OAuthProviderButton from '../components/oauth/OAuthProviderButton';
import { oauthProviderConfigs } from '../components/oauth/providerConfigs';
import RotatingTetrahedronCanvas from '../components/RotatingTetrahedronCanvas';
import Button from '../components/ui/Button';
import { useT } from '../lib/i18n/I18nContext';
import { clearBackendUrlCache } from '../services/backendUrl';
import { clearCoreRpcTokenCache, clearCoreRpcUrlCache } from '../services/coreRpcClient';
import { resetCoreMode } from '../store/coreModeSlice';
import { useDeepLinkAuthState } from '../store/deepLinkAuthState';
import { useAppDispatch } from '../store/hooks';
import { clearAllAppData } from '../utils/clearAllAppData';
import { clearStoredCoreMode, clearStoredCoreToken, storeRpcUrl } from '../utils/configPersistence';

const log = createDebug('app:welcome');

const Welcome = () => {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const { isProcessing, errorMessage, requiresAppDataReset } = useDeepLinkAuthState();

  const [isClearingAppData, setIsClearingAppData] = useState(false);
  const [resetError, setResetError] = useState<string | null>(null);

  const handleClearAppData = async () => {
    setIsClearingAppData(true);
    setResetError(null);
    try {
      // No live session at the Welcome screen — skip the core-side
      // `clearSession` step, just wipe local data and restart.
      await clearAllAppData();
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      log('clearAllAppData failed: %s', message);
      setResetError('Could not clear app data. Please quit and reopen OpenHuman, then try again.');
      setIsClearingAppData(false);
    }
  };

  const handleSelectRuntime = () => {
    log('[welcome] select-runtime — resetting core mode to return to picker');
    storeRpcUrl('');
    clearStoredCoreToken();
    clearStoredCoreMode();
    clearCoreRpcUrlCache();
    clearCoreRpcTokenCache();
    clearBackendUrlCache();
    dispatch(resetCoreMode());
  };

  return (
    <div className="min-h-full flex flex-col items-center justify-center p-4">
      <div className="max-w-md w-full">
        <div className="bg-white rounded-2xl shadow-soft border border-stone-200 p-8 animate-fade-up">
          <div className="flex justify-center mb-6">
            <div className="h-20 w-20">
              <RotatingTetrahedronCanvas />
            </div>
          </div>

          <h1 className="text-2xl font-bold text-stone-900 text-center mb-2">
            {t('welcome.title')}
          </h1>

          <p className="text-sm text-stone-500 text-center mb-6 leading-relaxed">
            {t('welcome.subtitle')}
          </p>

          {errorMessage ? (
            <div
              role="alert"
              className="mb-5 rounded-lg border border-red-200 bg-red-50 px-3 py-2 text-sm text-red-700">
              <p>{errorMessage}</p>
              {requiresAppDataReset ? (
                <div className="mt-3 space-y-2">
                  <button
                    type="button"
                    onClick={handleClearAppData}
                    disabled={isClearingAppData}
                    className="w-full rounded-lg bg-red-600 px-3 py-2 text-xs font-semibold text-white transition-colors hover:bg-red-700 disabled:cursor-not-allowed disabled:opacity-60">
                    {isClearingAppData ? (
                      <span className="flex items-center justify-center gap-2">
                        <span className="h-3 w-3 animate-spin rounded-full border border-white border-t-transparent" />
                        Clearing app data...
                      </span>
                    ) : (
                      'Clear app data & restart'
                    )}
                  </button>
                  <p className="text-[11px] leading-4 text-red-600/80">
                    This wipes locally stored secrets and accounts on this device. Your cloud
                    account is unaffected — you can sign in again right after.
                  </p>
                  {resetError ? (
                    <p className="text-[11px] leading-4 font-medium text-red-700">{resetError}</p>
                  ) : null}
                </div>
              ) : null}
            </div>
          ) : null}

          {isProcessing ? (
            <div
              role="status"
              aria-live="polite"
              aria-atomic="true"
              className="mb-5 flex flex-col items-center justify-center gap-3 py-2">
              <div className="h-6 w-6 animate-spin rounded-full border-2 border-stone-300 border-t-primary-500" />
              <p className="text-sm font-medium text-stone-700">Signing you in...</p>
            </div>
          ) : (
            <>
              {/* Real OAuth: click → system browser → backend → deep link back to app. */}
              <div className="flex items-center justify-center gap-3">
                {oauthProviderConfigs
                  .filter(provider => provider.showOnWelcome)
                  .map(provider => (
                    <OAuthProviderButton
                      key={provider.id}
                      provider={provider}
                      className="!rounded-full !px-4 !py-2"
                    />
                  ))}
              </div>
            </>
          )}
        </div>

        <div className="mt-4 px-2">
          <Button
            variant="secondary"
            size="md"
            onClick={handleSelectRuntime}
            className="w-full py-3">
            {t('welcome.selectRuntime')}
          </Button>
        </div>
      </div>
    </div>
  );
};

export default Welcome;
