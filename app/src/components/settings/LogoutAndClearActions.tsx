import debug from 'debug';
import { useId, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { useCoreState } from '../../providers/CoreStateProvider';
import { clearAllAppData } from '../../utils/clearAllAppData';
import Button from '../ui/Button';
import SettingsMenuItem from './components/SettingsMenuItem';

const warnLog = debug('settings:account:warn');

/**
 * Destructive account actions: Log out, and Log out + clear all app data.
 * Lives at the bottom of the Settings → Account page. Owns its own modal
 * state and confirmation flow so the parent page is just a list + this row.
 */
const LogoutAndClearActions = () => {
  const { t } = useT();
  const { clearSession, snapshot } = useCoreState();
  const [showLogoutAndClearModal, setShowLogoutAndClearModal] = useState(false);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const modalTitleId = useId();

  const handleLogout = async () => {
    try {
      await clearSession();
    } catch (err) {
      // Log only the message — `err` may carry stack frames / serialized
      // backend payloads we don't want in renderer console.
      const reason = err instanceof Error ? err.message : String(err);
      warnLog('logout_failed %o', { reason });
      setError(t('clearData.failedLogout'));
    }
  };

  const handleLogoutAndClearData = async () => {
    try {
      setIsLoading(true);
      setError(null);
      const currentUserId = snapshot.auth.userId ?? snapshot.currentUser?._id ?? null;
      await clearAllAppData({ clearSession, userId: currentUserId }); // restarts the app
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setError(message || t('clearData.failed'));
    } finally {
      setIsLoading(false);
    }
  };

  const arrowOutIcon = (
    <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={2}
        d="M17 16l4-4m0 0l-4-4m4 4H7m6 4v1a3 3 0 01-3 3H6a3 3 0 01-3-3V7a3 3 0 013-3h4a3 3 0 013 3v1"
      />
    </svg>
  );

  // Inline error is only displayed below the row when the clear-data modal is
  // closed — when the modal is open, it owns the error display. Without this
  // surface, a `handleLogout` failure would set `error` but the user would
  // never see it.
  const showInlineError = error !== null && !showLogoutAndClearModal;

  return (
    <div>
      <SettingsMenuItem
        icon={arrowOutIcon}
        title={t('settings.clearAppData')}
        description={t('settings.clearAppDataDesc')}
        onClick={() => setShowLogoutAndClearModal(true)}
        testId="settings-nav-logout-and-clear"
        dangerous
        isFirst
      />
      <SettingsMenuItem
        icon={arrowOutIcon}
        title={t('settings.logOut')}
        description={t('settings.logOutDesc')}
        onClick={handleLogout}
        testId="settings-nav-logout"
        dangerous
        isLast
      />

      {showInlineError && (
        <div
          role="alert"
          data-testid="logout-error"
          className="mt-3 mx-1 p-3 rounded-lg bg-coral-100 dark:bg-coral-500/20 border border-coral-500/20">
          <p className="text-coral-600 dark:text-coral-300 text-sm">{error}</p>
        </div>
      )}

      {showLogoutAndClearModal && (
        <div className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/30">
          <div
            role="dialog"
            aria-modal="true"
            aria-labelledby={modalTitleId}
            className="bg-surface rounded-2xl max-w-md w-full p-6 border border-line">
            <div className="flex items-center gap-3 mb-4">
              <div className="w-10 h-10 rounded-lg bg-amber-100 dark:bg-amber-500/20 flex items-center justify-center">
                <svg
                  className="w-5 h-5 text-amber-400"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24">
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M17 16l4-4m0 0l-4-4m4 4H7m6 4v1a3 3 0 01-3 3H6a3 3 0 01-3-3V7a3 3 0 013-3h4a3 3 0 013 3v1"
                  />
                </svg>
              </div>
              <div>
                <h3 id={modalTitleId} className="text-lg font-semibold text-content">
                  {t('clearData.title')}
                </h3>
              </div>
            </div>

            <div className="mb-6">
              <div className="text-content-secondary text-sm leading-relaxed">
                <p>{t('clearData.warning')}</p>
                <ul className="list-disc pl-5 mt-2 space-y-1">
                  <li>{t('clearData.bulletSettings')}</li>
                  <li>{t('clearData.bulletCache')}</li>
                  <li>{t('clearData.bulletWorkspace')}</li>
                  <li>{t('clearData.bulletOther')}</li>
                </ul>
                <p className="mt-3">{t('clearData.irreversible')}</p>
              </div>

              {error && (
                <div className="mt-3 p-3 rounded-lg bg-coral-100 dark:bg-coral-500/20 border border-coral-500/20">
                  <p className="text-coral-600 dark:text-coral-300 text-sm">{error}</p>
                </div>
              )}
            </div>

            <div className="flex gap-3">
              <Button
                variant="secondary"
                onClick={() => {
                  setShowLogoutAndClearModal(false);
                  setError(null);
                }}
                disabled={isLoading}
                className="flex-1">
                {t('common.cancel')}
              </Button>
              <button
                onClick={handleLogoutAndClearData}
                disabled={isLoading}
                className="flex-1 px-4 py-2 rounded-sm bg-amber-600 hover:bg-amber-500 text-content-inverted transition-colors disabled:opacity-50 flex items-center justify-center gap-2">
                {isLoading && (
                  <svg className="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24">
                    <circle
                      className="opacity-25"
                      cx="12"
                      cy="12"
                      r="10"
                      stroke="currentColor"
                      strokeWidth="4"
                    />
                    <path
                      className="opacity-75"
                      fill="currentColor"
                      d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
                    />
                  </svg>
                )}
                {isLoading ? t('clearData.clearing') : t('clearData.title')}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
};

export default LogoutAndClearActions;
