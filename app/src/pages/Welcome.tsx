import createDebug from 'debug';
import { useState } from 'react';

import OAuthProviderButton from '../components/oauth/OAuthProviderButton';
import { oauthProviderConfigs } from '../components/oauth/providerConfigs';
import RotatingTetrahedronCanvas from '../components/RotatingTetrahedronCanvas';
import { clearBackendUrlCache } from '../services/backendUrl';
import { clearCoreRpcUrlCache, testCoreRpcConnection } from '../services/coreRpcClient';
import { useDeepLinkAuthState } from '../store/deepLinkAuthState';
import { clearAllAppData } from '../utils/clearAllAppData';
import {
  clearStoredRpcUrl,
  getDefaultRpcUrl,
  getStoredRpcUrl,
  isValidRpcUrl,
  normalizeRpcUrl,
  storeRpcUrl,
} from '../utils/configPersistence';

const log = createDebug('app:welcome');

const Welcome = () => {
  const { isProcessing, errorMessage, requiresAppDataReset } = useDeepLinkAuthState();

  const [showAdvanced, setShowAdvanced] = useState(false);
  const [rpcUrl, setRpcUrl] = useState(getStoredRpcUrl());
  const [rpcUrlError, setRpcUrlError] = useState<string | null>(null);
  const [isTestingConnection, setIsTestingConnection] = useState(false);
  const [saveSuccess, setSaveSuccess] = useState(false);
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

  const handleRpcUrlChange = (value: string) => {
    setRpcUrl(value);
    setRpcUrlError(null);
    setSaveSuccess(false);
  };

  const handleSaveRpcUrl = () => {
    const normalized = normalizeRpcUrl(rpcUrl);

    if (!isValidRpcUrl(normalized)) {
      setRpcUrlError('Please enter a valid HTTP or HTTPS URL');
      return;
    }

    storeRpcUrl(normalized);
    clearCoreRpcUrlCache();
    clearBackendUrlCache();
    setRpcUrlError(null);
    setSaveSuccess(true);

    setTimeout(() => setSaveSuccess(false), 2000);
  };

  const handleResetRpcUrl = () => {
    clearStoredRpcUrl();
    clearCoreRpcUrlCache();
    clearBackendUrlCache();
    setRpcUrl(getDefaultRpcUrl());
    setRpcUrlError(null);
    setSaveSuccess(false);
  };

  const handleTestConnection = async () => {
    const normalized = normalizeRpcUrl(rpcUrl);

    if (!isValidRpcUrl(normalized)) {
      setRpcUrlError('Please enter a valid HTTP or HTTPS URL');
      return;
    }

    setIsTestingConnection(true);
    setRpcUrlError(null);

    try {
      const response = await testCoreRpcConnection(normalized);

      if (response.ok || response.status === 405) {
        setSaveSuccess(true);
        storeRpcUrl(normalized);
        clearCoreRpcUrlCache();
        clearBackendUrlCache();
      } else {
        setRpcUrlError(`Connection failed: ${response.status} ${response.statusText}`);
      }
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Unable to reach the RPC endpoint';
      setRpcUrlError(`Connection failed: ${message}`);
    } finally {
      setIsTestingConnection(false);
    }
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
            Sign in! Let's Cook
          </h1>

          <p className="text-sm text-stone-500 text-center mb-6 leading-relaxed">
            Welcome to <span className="font-medium text-stone-900">OpenHuman</span>! Your Personal
            AI Super Intelligence. Private, Simple and extremely powerful.
          </p>

          {showAdvanced ? (
            <div className="mb-5 p-4 bg-stone-50 rounded-xl border border-stone-200">
              <div className="flex items-center justify-between mb-3">
                <label htmlFor="rpc-url-input" className="text-xs font-medium text-stone-700">
                  Core RPC URL
                </label>
                <button
                  type="button"
                  onClick={() => setShowAdvanced(false)}
                  className="text-xs text-stone-500 hover:text-stone-700">
                  Close
                </button>
              </div>
              <div className="flex gap-2">
                <input
                  id="rpc-url-input"
                  type="url"
                  value={rpcUrl}
                  onChange={e => handleRpcUrlChange(e.target.value)}
                  placeholder="http://127.0.0.1:7788/rpc"
                  className="flex-1 rounded-lg border border-stone-300 bg-white px-3 py-2 text-xs text-stone-900 placeholder:text-stone-400 focus:border-primary-500 focus:outline-none focus:ring-1 focus:ring-primary-500"
                />
                <button
                  type="button"
                  onClick={handleTestConnection}
                  disabled={isTestingConnection}
                  className="px-3 py-2 bg-stone-200 hover:bg-stone-300 text-stone-700 text-xs font-medium rounded-lg transition-colors disabled:opacity-60">
                  {isTestingConnection ? (
                    <span className="flex items-center gap-1">
                      <span className="h-3 w-3 animate-spin rounded-full border border-stone-400 border-t-transparent" />
                      Testing
                    </span>
                  ) : (
                    'Test'
                  )}
                </button>
              </div>
              {rpcUrlError ? (
                <p className="mt-2 text-xs text-red-600">{rpcUrlError}</p>
              ) : saveSuccess ? (
                <p className="mt-2 text-xs text-green-600">URL saved successfully.</p>
              ) : null}
              <div className="mt-3 flex gap-2">
                <button
                  type="button"
                  onClick={handleSaveRpcUrl}
                  className="px-3 py-1.5 bg-primary-500 hover:bg-primary-600 text-white text-xs font-medium rounded-lg transition-colors">
                  Save
                </button>
                <button
                  type="button"
                  onClick={handleResetRpcUrl}
                  className="px-3 py-1.5 bg-stone-200 hover:bg-stone-300 text-stone-700 text-xs font-medium rounded-lg transition-colors">
                  Reset to Default
                </button>
              </div>
            </div>
          ) : (
            <button
              type="button"
              onClick={() => setShowAdvanced(true)}
              className="mb-5 text-xs text-stone-500 hover:text-stone-700 underline">
              Configure RPC URL (Advanced)
            </button>
          )}

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
      </div>
    </div>
  );
};

export default Welcome;
