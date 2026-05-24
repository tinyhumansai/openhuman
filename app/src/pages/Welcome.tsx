import createDebug from 'debug';
import { useState } from 'react';
import { useNavigate } from 'react-router-dom';

import OAuthProviderButton from '../components/oauth/OAuthProviderButton';
import { oauthProviderConfigs } from '../components/oauth/providerConfigs';
import RotatingTetrahedronCanvas from '../components/RotatingTetrahedronCanvas';
import Button from '../components/ui/Button';
import { useT } from '../lib/i18n/I18nContext';
import { useCoreState } from '../providers/CoreStateProvider';
import { clearBackendUrlCache } from '../services/backendUrl';
import { clearCoreRpcTokenCache, clearCoreRpcUrlCache } from '../services/coreRpcClient';
import { resetCoreMode } from '../store/coreModeSlice';
import { useDeepLinkAuthState } from '../store/deepLinkAuthState';
import { useAppDispatch, useAppSelector } from '../store/hooks';
import { resolveTheme, setThemeMode, type ThemeMode } from '../store/themeSlice';
import { clearAllAppData } from '../utils/clearAllAppData';
import { clearStoredCoreMode, clearStoredCoreToken, storeRpcUrl } from '../utils/configPersistence';
import { PRIVACY_POLICY_URL, TERMS_OF_USE_URL } from '../utils/links';
import { createLocalSessionToken, LOCAL_SESSION_USER } from '../utils/localSession';
import { openUrl } from '../utils/openUrl';

const log = createDebug('app:welcome');

const Welcome = () => {
  const { t } = useT();
  const navigate = useNavigate();
  const dispatch = useAppDispatch();
  const { storeSessionToken } = useCoreState();
  const { isProcessing, errorMessage, requiresAppDataReset } = useDeepLinkAuthState();
  const themeMode = useAppSelector(state => state.theme?.mode ?? 'system') as ThemeMode;
  const resolvedTheme = resolveTheme(themeMode);
  const isDark = resolvedTheme === 'dark';

  const [isClearingAppData, setIsClearingAppData] = useState(false);
  const [isLocalSigningIn, setIsLocalSigningIn] = useState(false);
  const [resetError, setResetError] = useState<string | null>(null);
  const [localLoginError, setLocalLoginError] = useState<string | null>(null);

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
      setResetError(message || t('welcome.resetErrorFallback'));
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

  const handleLocalLogin = async () => {
    setIsLocalSigningIn(true);
    setLocalLoginError(null);
    try {
      log('[welcome] local session login requested');
      await storeSessionToken(createLocalSessionToken(), LOCAL_SESSION_USER);
      navigate('/onboarding/custom/inference', { replace: true });
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      log('[welcome] local session login failed: %s', message);
      setLocalLoginError(message || 'Could not start a local session.');
      setIsLocalSigningIn(false);
    }
  };

  const toggleTheme = () => {
    dispatch(setThemeMode(isDark ? 'light' : 'dark'));
  };

  return (
    <div className="min-h-full flex flex-col items-center justify-center p-4">
      <div className="max-w-md w-full">
        <div className="bg-white dark:bg-neutral-900 rounded-2xl shadow-soft border border-stone-200 dark:border-neutral-800 p-8 animate-fade-up">
          <div className="flex items-center justify-between mb-4">
            <div className="w-9" aria-hidden="true" />
            <div className="w-9" aria-hidden="true" />
            <button
              type="button"
              onClick={toggleTheme}
              aria-label={isDark ? t('home.themeToggle.toLight') : t('home.themeToggle.toDark')}
              title={isDark ? t('home.themeToggle.toLight') : t('home.themeToggle.toDark')}
              className="p-2 rounded-full text-stone-500 dark:text-neutral-400 hover:text-stone-700 dark:hover:text-neutral-200 hover:bg-stone-100 dark:hover:bg-neutral-800/60 transition-colors">
              {isDark ? (
                <svg
                  className="w-5 h-5"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth={2}
                  viewBox="0 0 24 24"
                  aria-hidden="true">
                  <circle cx="12" cy="12" r="4" />
                  <path
                    strokeLinecap="round"
                    d="M12 2v2M12 20v2M4.93 4.93l1.41 1.41M17.66 17.66l1.41 1.41M2 12h2M20 12h2M4.93 19.07l1.41-1.41M17.66 6.34l1.41-1.41"
                  />
                </svg>
              ) : (
                <svg
                  className="w-5 h-5"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth={2}
                  viewBox="0 0 24 24"
                  aria-hidden="true">
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79Z"
                  />
                </svg>
              )}
            </button>
          </div>
          <div className="flex justify-center mb-6">
            <div className="h-20 w-20">
              <RotatingTetrahedronCanvas />
            </div>
          </div>

          <h1 className="text-2xl font-bold text-stone-900 dark:text-neutral-100 text-center mb-2">
            {t('welcome.title')}
          </h1>

          <p className="text-sm text-stone-500 dark:text-neutral-400 text-center mb-6 leading-relaxed">
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
                        {t('welcome.clearingAppData')}
                      </span>
                    ) : (
                      t('welcome.clearAppDataAndRestart')
                    )}
                  </button>
                  <p className="text-[11px] leading-4 text-red-600/80">
                    {t('welcome.clearAppDataWarning')}
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
              <div className="h-6 w-6 animate-spin rounded-full border-2 border-stone-300 dark:border-neutral-700 border-t-primary-500" />
              <p className="text-sm font-medium text-stone-700 dark:text-neutral-200">
                {t('welcome.signingIn')}
              </p>
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
              <p className="mt-5 text-center text-[11px] leading-5 text-stone-500 dark:text-neutral-500">
                {t('welcome.termsIntro')}{' '}
                <a
                  href={TERMS_OF_USE_URL}
                  target="_blank"
                  rel="noreferrer"
                  onClick={event => {
                    event.preventDefault();
                    void openUrl(TERMS_OF_USE_URL);
                  }}
                  className="font-medium text-stone-700 underline underline-offset-2 hover:text-stone-900 dark:text-neutral-300 dark:hover:text-neutral-100">
                  {t('welcome.termsOfUse')}
                </a>{' '}
                {t('welcome.termsJoiner')}{' '}
                <a
                  href={PRIVACY_POLICY_URL}
                  target="_blank"
                  rel="noreferrer"
                  onClick={event => {
                    event.preventDefault();
                    void openUrl(PRIVACY_POLICY_URL);
                  }}
                  className="font-medium text-stone-700 underline underline-offset-2 hover:text-stone-900 dark:text-neutral-300 dark:hover:text-neutral-100">
                  {t('welcome.privacyPolicy')}
                </a>
                {t('welcome.termsOutro')}
              </p>
            </>
          )}
        </div>

        <div className="mt-4 px-2 space-y-2">
          <Button
            variant="secondary"
            size="md"
            onClick={handleSelectRuntime}
            className="w-full py-3">
            {t('welcome.selectRuntime')}
          </Button>
          <Button
            variant="secondary"
            size="md"
            onClick={handleLocalLogin}
            disabled={isLocalSigningIn}
            className="w-full py-3">
            {isLocalSigningIn
              ? t('welcome.localSessionStarting')
              : t('welcome.continueLocallyExperimental')}
          </Button>
          {localLoginError ? (
            <p className="text-[11px] leading-4 text-center font-medium text-red-700">
              {localLoginError}
            </p>
          ) : null}
        </div>
      </div>
    </div>
  );
};

export default Welcome;
