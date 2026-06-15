// Settings → Developer Options → Skills Runner — thin wrapper around the
// reusable `<WorkflowRunnerBody />` so the settings shell (header + back
// button + breadcrumbs) stays consistent with other panels. The actual
// picker / Run / Schedule / Recent Runs UX lives in
// `app/src/components/skills/WorkflowRunnerBody.tsx`, shared with the
// top-level /skills page's "Runners" tab.
import { useT } from '../../../lib/i18n/I18nContext';
import PanelPage from '../../layout/PanelPage';
import WorkflowRunnerBody from '../../skills/WorkflowRunnerBody';
import SettingsBackButton from '../components/SettingsBackButton';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const WorkflowRunnerPanel = () => {
  const { t } = useT();
  const { navigateBack } = useSettingsNavigation();

  return (
    <PanelPage
      className="z-10"
      contentClassName=""
      description={t('settings.developerMenu.skillsRunner.desc')}
      leading={<SettingsBackButton onBack={navigateBack} />}>
      <div className="flex-1 overflow-y-auto p-6">
        <WorkflowRunnerBody />
      </div>
    </PanelPage>
  );
};

export default WorkflowRunnerPanel;
