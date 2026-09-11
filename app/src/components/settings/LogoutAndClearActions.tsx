import debug from 'debug';
import { useId, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { useCoreState } from '../../providers/CoreStateProvider';
import { clearAllAppData } from '../../utils/clearAllAppData';
import Button from '../ui/Button';
import { Spinner } from '../ui/icons';
import { ModalShell } from '../ui/ModalShell';
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

  // Two actions, two glyphs. They shared one arrow-out icon, which is most of
  // why an irreversible wipe and a routine sign-out read as the same control.
  const signOutIcon = (
    <svg className="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={2}
        d="M17 16l4-4m0 0l-4-4m4 4H7m6 4v1a3 3 0 01-3 3H6a3 3 0 01-3-3V7a3 3 0 013-3h4a3 3 0 013 3v1"
      />
    </svg>
  );

  const eraseIcon = (
    <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={2}
        d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"
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
      {/* Log out is routine and reversible, so it reads as an ordinary row.
          It was `dangerous` and amber, identical to the wipe below it. */}
      <SettingsMenuItem
        icon={signOutIcon}
        title={t('settings.logOut')}
        description={t('settings.logOutDesc')}
        onClick={handleLogout}
        testId="settings-nav-logout"
        isFirst
        isLast
      />

      {/* Clearing app data is not a peer of logging out, so it stops being a
          row in the same menu. Separating it is the fix: the two were the same
          amber, the same weight and the same icon, one row apart, and amber is
          this app's WARNING ramp -- coral is the destructive one. Placement,
          ramp, glyph and an explicit consequence line now all say the same
          thing, and the action sits behind a confirm. */}
      <section
        className="mt-5 rounded-xl border border-coral-500/30 bg-coral-500/[0.06] p-4"
        data-testid="account-destructive-zone">
        <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
          <div className="max-w-[52ch]">
            <h3 className="font-title text-sm font-semibold text-content">
              {t('settings.clearAppData')}
            </h3>
            <p className="mt-1 text-xs leading-relaxed text-content-muted">
              {t('settings.clearAppDataDesc')}
            </p>
            <p className="mt-1 text-xs font-medium text-coral-600 dark:text-coral-300">
              {t('settings.clearAppDataIrreversible')}
            </p>
          </div>
          <Button
            variant="secondary"
            tone="danger"
            size="sm"
            leadingIcon={eraseIcon}
            onClick={() => setShowLogoutAndClearModal(true)}
            data-testid="settings-nav-logout-and-clear"
            className="shrink-0">
            {t('settings.clearAppDataAction')}
          </Button>
        </div>
      </section>

      {showInlineError && (
        <div
          role="alert"
          data-testid="logout-error"
          className="mt-3 mx-1 p-3 rounded-lg bg-coral-100 dark:bg-coral-500/20 border border-coral-500/20">
          <p className="text-coral-600 dark:text-coral-300 text-sm">{error}</p>
        </div>
      )}

      {showLogoutAndClearModal && (
        <ModalShell
          title={t('clearData.title')}
          titleId={modalTitleId}
          icon={eraseIcon}
          onClose={() => {
            setShowLogoutAndClearModal(false);
            setError(null);
          }}
          contentClassName="px-6 py-5"
          closePolicy={isLoading ? { escape: false, backdrop: false, button: false } : undefined}
          footer={
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
              {/* Was an amber wash painted over `variant="tertiary"`, which
                  bypassed the danger tone entirely and used the warning ramp
                  for the most destructive confirm in the app. */}
              <Button
                variant="primary"
                tone="danger"
                onClick={handleLogoutAndClearData}
                disabled={isLoading}
                leadingIcon={isLoading ? <Spinner /> : undefined}
                className="flex-1">
                {isLoading ? t('clearData.clearing') : t('clearData.title')}
              </Button>
            </div>
          }>
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
        </ModalShell>
      )}
    </div>
  );
};

export default LogoutAndClearActions;
