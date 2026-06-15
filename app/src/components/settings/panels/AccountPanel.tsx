import { useT } from '../../../lib/i18n/I18nContext';
import { useCoreState } from '../../../providers/CoreStateProvider';
import PanelPage from '../../layout/PanelPage';
import SettingsBackButton from '../components/SettingsBackButton';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import LogoutAndClearActions from '../LogoutAndClearActions';

/**
 * Account landing page for the two-pane settings layout. The old Account hub
 * list (Team / Privacy / Security / Migration) is replaced by the sub-nav
 * pills above the panel; this page keeps the signed-in summary and the
 * destructive logout/clear actions.
 */
const AccountPanel = () => {
  const { t } = useT();
  const { navigateBack } = useSettingsNavigation();
  const { snapshot } = useCoreState();

  const user = snapshot.currentUser;
  const name = user ? [user.firstName, user.lastName].filter(Boolean).join(' ') || null : null;
  const username = user?.username ? `@${user.username}` : null;

  return (
    <PanelPage
      className="z-10"
      testId="account-panel"
      description={t('pages.settings.accountSection.description')}
      leading={<SettingsBackButton onBack={navigateBack} />}>
      {(name || username) && (
        <div className="flex items-center gap-3 rounded-2xl border border-stone-200 dark:border-neutral-800 px-4 py-3">
          <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-full bg-primary-100 dark:bg-primary-500/15 text-sm font-semibold text-primary-700 dark:text-primary-300">
            {(name ?? username ?? '?').replace('@', '').slice(0, 1).toUpperCase()}
          </div>
          <div className="min-w-0">
            {name && (
              <div className="truncate text-sm font-medium text-stone-900 dark:text-neutral-100">
                {name}
              </div>
            )}
            {username && (
              <div className="truncate text-xs text-stone-500 dark:text-neutral-400">
                {username}
              </div>
            )}
          </div>
        </div>
      )}

      <div className="rounded-2xl overflow-hidden border border-stone-200 dark:border-neutral-800">
        <LogoutAndClearActions />
      </div>
    </PanelPage>
  );
};

export default AccountPanel;
