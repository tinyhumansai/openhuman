// Settings → Developer Options → Skills Runner — thin wrapper around the
// reusable `<WorkflowRunnerBody />` so the settings shell (header + back
// button + breadcrumbs) stays consistent with other panels. The actual
// picker / Run / Schedule / Recent Runs UX lives in
// `app/src/components/skills/WorkflowRunnerBody.tsx`, shared with the
// top-level /skills page's "Runners" tab.
import { useT } from '../../../lib/i18n/I18nContext';
import WorkflowRunnerBody from '../../skills/WorkflowRunnerBody';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const WorkflowRunnerPanel = () => {
  const { t } = useT();
  const { navigateBack, breadcrumbs } = useSettingsNavigation();

  return (
    <div className="z-10 relative flex flex-col h-full">
      <SettingsHeader
        title={t('settings.developerMenu.skillsRunner.title')}
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />
      <div className="flex-1 overflow-y-auto p-6">
        <WorkflowRunnerBody />
      </div>
    </div>
  );
};

export default WorkflowRunnerPanel;
