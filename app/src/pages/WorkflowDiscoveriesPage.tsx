/**
 * WorkflowDiscoveriesPage — the dedicated home for Flow Scout's proactive,
 * buildable workflow suggestions. Previously these rendered inline on the
 * Workflows list page; they now live on their own sidebar-reachable page so the
 * list stays focused on the user's saved workflows.
 */
import SuggestedWorkflows from '../components/flows/SuggestedWorkflows';
import SettingsTabbedPage from '../components/settings/layout/SettingsTabbedPage';
import { useT } from '../lib/i18n/I18nContext';

export default function WorkflowDiscoveriesPage() {
  const { t } = useT();
  return (
    <div className="h-full p-4">
      <SettingsTabbedPage
        title={t('flows.discoveries.title')}
        description={t('flows.discoveries.description')}>
        {/* No wrapper padding: SettingsTabbedPage's body already renders
            `min-h-full pb-4 pt-4`, so a `pt-4` here double-pads the top. */}
        <SuggestedWorkflows />
      </SettingsTabbedPage>
    </div>
  );
}
