/**
 * /workflows/run — single-purpose workflow runner page.
 *
 * Reached by clicking a workflow card (which locks the page to that
 * workflow via `?workflow=<id>&lock=1`) or any `?workflow=<id>` deep link.
 * Hosts the WorkflowRunnerBody picker + form + run-now + edit + schedule +
 * recent-runs flow without the Connections-page tab chrome.
 *
 * Bookmark-friendly and shareable via `?workflow=<id>` (the body reads the
 * query param and pre-selects the workflow — see WorkflowRunnerBody.tsx).
 */
import { useNavigate } from 'react-router-dom';

import SettingsTabbedPage from '../components/settings/layout/SettingsTabbedPage';
import WorkflowRunnerBody from '../components/skills/WorkflowRunnerBody';
import Button from '../components/ui/Button';
import Card from '../components/ui/Card';
import { useT } from '../lib/i18n/I18nContext';

export default function WorkflowsRun() {
  const { t } = useT();
  const navigate = useNavigate();

  return (
    <div className="h-full p-4">
      <SettingsTabbedPage
        title={t('skills.run.title')}
        /* `leading` (not `headerAction`) is the slot for a back control: it
           sits before the title on the same row, which is where every settings
           page puts its back button. */
        leading={
          <Button
            variant="tertiary"
            size="xs"
            onClick={() =>
              (window.history.state?.idx ?? 0) > 0 ? navigate(-1) : navigate('/flows')
            }
            aria-label={t('common.back')}>
            <span aria-hidden="true">←</span> {t('common.back')}
          </Button>
        }>
        <Card className="animate-fade-up">
          <div className="p-4">
            <WorkflowRunnerBody />
          </div>
        </Card>
      </SettingsTabbedPage>
    </div>
  );
}
