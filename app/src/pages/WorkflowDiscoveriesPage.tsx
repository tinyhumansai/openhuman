/**
 * WorkflowDiscoveriesPage — the dedicated home for Flow Scout's proactive,
 * buildable workflow suggestions. Previously these rendered inline on the
 * Workflows list page; they now live on their own sidebar-reachable page so the
 * list stays focused on the user's saved workflows.
 */
import SuggestedWorkflows from '../components/flows/SuggestedWorkflows';
import PanelPage from '../components/layout/PanelPage';
import { useT } from '../lib/i18n/I18nContext';

export default function WorkflowDiscoveriesPage() {
  const { t } = useT();
  return (
    <PanelPage
      testId="workflow-discoveries-page"
      title={t('flows.discoveries.title')}
      description={t('flows.discoveries.description')}>
      <div className="p-4">
        <SuggestedWorkflows />
      </div>
    </PanelPage>
  );
}
