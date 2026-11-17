import createDebug from 'debug';
import { useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import LanguageSelect from '../components/LanguageSelect';
import OAuthProviderButton from '../components/oauth/OAuthProviderButton';
import { oauthProviderConfigs } from '../components/oauth/providerConfigs';
import { Alert, AlertDescription, Button, Card } from '../components/ui';
import { useT } from '../lib/i18n/I18nContext';
import { useCoreState } from '../providers/CoreStateProvider';
import { clearBackendUrlCache } from '../services/backendUrl';
import { clearCoreRpcTokenCache, clearCoreRpcUrlCache } from '../services/coreRpcClient';
import { resetCoreMode } from '../store/coreModeSlice';
import {
  endAwaitingAuthCallback,
  getDeepLinkAuthState,
  useDeepLinkAuthState,
} from '../store/deepLinkAuthState';
import { useAppDispatch, useAppSelector } from '../store/hooks';
import { resolveTheme, setThemeMode, type ThemeMode } from '../store/themeSlice';
import { getActiveUserId } from '../store/userScopedStorage';
import { clearAllAppData } from '../utils/clearAllAppData';
import { clearStoredCoreMode, clearStoredCoreToken, storeRpcUrl } from '../utils/configPersistence';
import {
  CORE_CONFIG_UNREADABLE_I18N_KEY,
  isCoreConfigUnreadableError,
} from '../utils/coreConfigFailure';
import { PRIVACY_POLICY_URL, TERMS_OF_USE_URL } from '../utils/links';
import { createLocalSessionToken, LOCAL_SESSION_USER } from '../utils/localSession';
import { openUrl } from '../utils/openUrl';

const log = createDebug('app:welcome');

/** The sign-in buttons, shared by the TinyHumans card and the hand-off panel. */
const ProviderButtons = ({ localProfileId }: { localProfileId: string | null }) => (
  <div className="flex flex-wrap items-center justify-center gap-3">
    {oauthProviderConfigs
      .filter(provider => provider.showOnWelcome)
      .map(provider => (
        <OAuthProviderButton
          key={provider.id}
          provider={provider}
          className="rounded-full!"
          localProfileId={localProfileId}
        />
      ))}
  </div>
);

/**
 * How long a regained focus waits for a deep link before concluding there
 * isn't one. The callback is delivered after the OS focus event, so this has
 * to outlast that gap; it only ever delays *giving up*, never a success.
 */
const CALLBACK_GRACE_MS = 1_500;

const Welcome = () => {
  const { t } = useT();
  const navigate = useNavigate();
  const dispatch = useAppDispatch();
  const { storeSessionToken } = useCoreState();
  const { isProcessing, awaitingCallback, errorMessage, errorMessageKey, requiresAppDataReset } =
    useDeepLinkAuthState();
  // Deep-link auth runs outside React and cannot translate its own copy, so it
  // hands over a key for the failures whose copy is localized. Everything else
  // still carries a literal message.
  const deepLinkError = errorMessageKey ? t(errorMessageKey) : errorMessage;
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
      // A config-read denial is permanent and host-side; showing the raw
      // anyhow chain (absolute path + `os error 13`) gives the user nothing to
      // act on. Unrecognized failures keep their original message.
      setLocalLoginError(
        isCoreConfigUnreadableError(message)
          ? t(CORE_CONFIG_UNREADABLE_I18N_KEY)
          : message || t('welcome.localSessionErrorFallback')
      );
      setIsLocalSigningIn(false);
    }
  };

  // Coming back to the app without a callback means the user cancelled, closed
  // the tab, or the redirect failed. End the wait so they get the cards back
  // instead of a spinner that only the 5-minute timeout would clear.
  //
  // The check cannot be synchronous. On a *successful* sign-in the OS focus
  // event arrives BEFORE the deep link does (see the same race documented at
  // `OAuthProviderButton`'s `skipDuringDeepLink`), so reading `isProcessing`
  // the instant focus fires would say "no callback" during a perfectly good
  // round-trip and flash the cards back mid-sign-in. Give the callback a grace
  // window to land, then decide.
  useEffect(() => {
    if (!awaitingCallback) return;
    let graceTimer: ReturnType<typeof setTimeout> | null = null;
    const onFocus = () => {
      if (graceTimer !== null) return;
      graceTimer = setTimeout(() => {
        graceTimer = null;
        const state = getDeepLinkAuthState();
        // Redeeming a callback, or something already ended the wait: leave it.
        if (state.isProcessing || !state.awaitingCallback) return;
        endAwaitingAuthCallback();
      }, CALLBACK_GRACE_MS);
    };
    window.addEventListener('focus', onFocus);
    return () => {
      if (graceTimer !== null) clearTimeout(graceTimer);
      window.removeEventListener('focus', onFocus);
    };
  }, [awaitingCallback]);

  const toggleTheme = () => {
    dispatch(setThemeMode(isDark ? 'light' : 'dark'));
  };

  const features = [
    'welcome.th.featureInference',
    'welcome.th.featureSearch',
    'welcome.th.featureVoice',
    'welcome.th.featureMemory',
    'welcome.th.featureEmbeddings',
    'welcome.th.featureBilling',
  ] as const;
  const steps = ['welcome.self.step1', 'welcome.self.step2', 'welcome.self.step3'] as const;

  const legalLink = (href: string, label: string) => (
    <a
      href={href}
      target="_blank"
      rel="noreferrer"
      onClick={event => {
        event.preventDefault();
        void openUrl(href);
      }}
      className="font-medium text-content-secondary underline underline-offset-2 hover:text-content">
      {label}
    </a>
  );

  return (
    <div className="min-h-full flex flex-col items-center justify-center p-4">
      <div className="w-full max-w-3xl animate-fade-up">
        <div className="flex items-center justify-end gap-2">
          {/* Language sits with the other presentation control rather than on
              the boot-check panel, which is now a failure-only fallback most
              people never see. */}
          <LanguageSelect id="welcome-language" ariaLabel={t('settings.language')} />
          <Button
            iconOnly
            variant="tertiary"
            onClick={toggleTheme}
            aria-label={isDark ? t('home.themeToggle.toLight') : t('home.themeToggle.toDark')}
            title={isDark ? t('home.themeToggle.toLight') : t('home.themeToggle.toDark')}
            className="rounded-full">
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
          </Button>
        </div>

        <div className="flex flex-col items-center text-center">
          <div className="flex items-center gap-3">
            <img
              src={isDark ? '/brand/OpenhumanLogo-white.svg' : '/brand/OpenhumanLogo-Black.svg'}
              alt={t('welcome.logoAlt')}
              className="h-12 w-12"
            />
            <span className="text-xl font-bold text-content">OpenHuman</span>
          </div>
          <h1 className="mt-4 text-2xl font-bold text-content">{t('welcome.title')}</h1>
          <p className="mt-2 text-sm leading-relaxed text-content-muted">
            {t('welcome.hero.subtitle')}
          </p>
        </div>

        {deepLinkError ? (
          <Alert variant="destructive" className="mt-6">
            <AlertDescription>
              <p className="font-medium">{t('welcome.handoff.failedTitle')}</p>
              <p className="mt-1">{deepLinkError}</p>
              {/* Sign-in that never comes back is a dead end unless the screen
                  offers somewhere to go. Both doors: try the browser again, or
                  take the self-hosted path instead. */}
              {!requiresAppDataReset ? (
                <div className="mt-3 flex flex-wrap gap-2">
                  {/* No provider buttons here: the TinyHumans card below shows
                      them permanently now, so repeating them put two Google
                      buttons on screen. Retrying is clicking one of those. */}
                  <Button
                    variant="secondary"
                    size="sm"
                    onClick={() => void handleLocalLogin()}
                    disabled={isLocalSigningIn}
                    data-testid="welcome-handoff-fallback-self">
                    {t('welcome.handoff.fallbackSelf')}
                  </Button>
                </div>
              ) : null}
              {requiresAppDataReset ? (
                <div className="mt-3 space-y-2">
                  <Button
                    variant="primary"
                    tone="danger"
                    size="sm"
                    onClick={handleClearAppData}
                    disabled={isClearingAppData}
                    className="w-full">
                    {isClearingAppData ? (
                      <span className="flex items-center justify-center gap-2">
                        <span className="h-3 w-3 animate-spin rounded-full border border-content-inverted border-t-transparent" />
                        {t('welcome.clearingAppData')}
                      </span>
                    ) : (
                      t('welcome.clearAppDataAndRestart')
                    )}
                  </Button>
                  <p className="text-[11px] leading-4">{t('welcome.clearAppDataWarning')}</p>
                  {resetError ? (
                    <p className="text-[11px] leading-4 font-medium">{resetError}</p>
                  ) : null}
                </div>
              ) : null}
            </AlertDescription>
          </Alert>
        ) : null}

        {/* `isProcessing` alone is wrong here: it covers an auth STEP, and the
            launch step ends the instant the browser opens — so the hand-off
            appeared for about a second and then dropped the user back to the
            sign-in buttons while they were still in the browser.
            `awaitingCallback` is the actual wait. */}
        {isProcessing || awaitingCallback ? (
          /* Screen B — the browser hand-off. Sign-in leaves the app entirely,
             so the window the user comes back to has to say where they are and
             give them a way out. A bare spinner said neither. */
          <Card padded divided={false} className="mt-8 shadow-soft" data-testid="welcome-handoff">
            <div
              role="status"
              aria-live="polite"
              aria-atomic="true"
              className="flex flex-col items-center gap-4 py-4 text-center">
              <div className="h-7 w-7 animate-spin rounded-full border-2 border-line-strong border-t-primary-500" />
              <div>
                <p className="text-base font-semibold text-content">{t('welcome.handoff.title')}</p>
                <p className="mt-1 text-sm text-content-muted">{t('welcome.handoff.body')}</p>
              </div>
              {/* No sign-in buttons here. By this point a provider has been
                  picked and the browser is open; repeating the row just asks
                  the same question twice. If the browser never comes back, the
                  90s timeout swaps this panel for the failure state, which is
                  where the way out belongs. */}
            </div>
          </Card>
        ) : (
          <>
            {/* Equal columns. The two paths are a genuine either/or, so neither
                card is sized to argue for itself; the TinyHumans one leads by
                being first and denser. Stacks to one column at phone width. */}
            <div className="mt-8 grid grid-cols-1 items-stretch gap-4 sm:grid-cols-2">
              <Card
                data-testid="welcome-card-tinyhumans"
                padded
                divided={false}
                className="flex flex-col shadow-soft">
                <div className="flex h-full flex-col gap-4">
                  <div>
                    <h2 className="text-lg font-semibold text-content">{t('welcome.th.title')}</h2>
                    <p className="mt-1 text-sm text-content-muted">{t('welcome.th.promise')}</p>
                  </div>
                  <ul className="grid grid-cols-2 gap-x-4 gap-y-2 text-sm text-content-secondary">
                    {features.map(key => (
                      <li key={key} className="flex items-center gap-2">
                        <span
                          aria-hidden="true"
                          className="h-2 w-2 shrink-0 rotate-45 rounded-[2px] bg-primary-500"
                        />
                        {t(key)}
                      </li>
                    ))}
                  </ul>
                  <div className="mt-auto space-y-3">
                    {/* TODO: replace this inline provider reveal with a single `openUrl` to a
                        tinyhumans.ai login page that returns
                        `openhuman://auth?token=...&state=...`. That hosted single-login page
                        does not exist yet, so for now the CTA reveals the three existing
                        OAuth provider buttons (google, github, twitter). */}
                    {/* The providers are the action, so they are on screen.
                        A "Continue with TinyHumans" button used to sit here and
                        reveal them on click, which was a tap that bought the
                        user nothing.

                        WHEN THE HOSTED LOGIN LANDS: put that single button back
                        in place of this row and have it `openUrl` to the
                        tinyhumans.ai login page with `redirectUri` +`state`;
                        the page returns `openhuman://auth?token=…&state=…`,
                        which `desktopDeepLinkListener` already redeems through
                        `loginWithToken` → `POST /auth/login-token/consume`.
                        Every piece of that exists except the page itself, which
                        is why provider selection is still in the app. */}
                    <p className="text-center text-xs text-content-muted">
                      {t('welcome.th.providers')}
                    </p>
                    <div data-testid="welcome-cta-tinyhumans">
                      <ProviderButtons localProfileId={getActiveUserId()} />
                    </div>
                  </div>
                </div>
              </Card>

              <Card
                data-testid="welcome-card-self"
                padded
                divided={false}
                className="flex flex-col shadow-soft">
                <div className="flex h-full flex-col gap-4">
                  <div>
                    <h2 className="text-lg font-semibold text-content">
                      {t('welcome.self.title')}
                    </h2>
                    <p className="mt-1 text-sm text-content-muted">{t('welcome.self.promise')}</p>
                  </div>
                  <div>
                    <p className="text-sm font-medium text-content-secondary">
                      {t('welcome.self.listLabel')}
                    </p>
                    <ol className="mt-2 space-y-2 text-sm text-content-secondary">
                      {steps.map((key, index) => (
                        <li key={key} className="flex items-center gap-3">
                          <span
                            aria-hidden="true"
                            className="flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-surface-muted text-xs font-semibold text-content-secondary">
                            {index + 1}
                          </span>
                          {t(key)}
                        </li>
                      ))}
                    </ol>
                  </div>
                  <p className="text-xs text-content-muted">{t('welcome.self.time')}</p>
                  <div className="mt-auto space-y-2">
                    <Button
                      data-testid="welcome-cta-self"
                      variant="secondary"
                      size="md"
                      onClick={handleLocalLogin}
                      disabled={isLocalSigningIn}
                      className="w-full py-3">
                      {isLocalSigningIn ? t('welcome.localSessionStarting') : t('welcome.self.cta')}
                    </Button>
                    {localLoginError ? (
                      <p
                        role="alert"
                        className="text-center text-[11px] leading-4 font-medium text-coral-600">
                        {localLoginError}
                      </p>
                    ) : null}
                  </div>
                </div>
              </Card>
            </div>

            {/* A door for the few, not a third option: sits below both cards. */}
            <p className="mt-6 text-center text-sm text-content-muted">
              {t('welcome.serverPrompt')}{' '}
              <button
                type="button"
                data-testid="welcome-server-link"
                onClick={handleSelectRuntime}
                className="font-medium text-primary-600 underline underline-offset-2 hover:text-primary-700">
                {t('welcome.serverCta')}
              </button>
            </p>

            <p className="mt-4 text-center text-[11px] leading-5 text-content-muted dark:text-content-faint">
              {t('welcome.termsIntro')} {legalLink(TERMS_OF_USE_URL, t('welcome.termsOfUse'))}{' '}
              {t('welcome.termsJoiner')} {legalLink(PRIVACY_POLICY_URL, t('welcome.privacyPolicy'))}
              {t('welcome.termsOutro')}
            </p>
          </>
        )}
      </div>
    </div>
  );
};

export default Welcome;
