import createDebug from 'debug';
import { useCallback, useEffect, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import {
  type CoreCronJob,
  type CoreCronRun,
  type CronAddParams,
  openhumanCronAdd,
  openhumanCronList,
  openhumanCronRemove,
  openhumanCronRun,
  openhumanCronRuns,
  openhumanCronUpdate,
} from '../../../utils/tauriCommands';
import Button from '../../ui/Button';
import { SettingsSection, SettingsStatusLine } from '../controls';
import SettingsPanel from '../layout/SettingsPanel';
import CoreJobList from './cron/CoreJobList';
import CronJobFormModal from './cron/CronJobFormModal';

const loadCronJobsLog = createDebug('app:settings:CronJobsPanel:loadCronSkills');

const CronJobsPanel = () => {
  const { t } = useT();
  const formatCronError = useCallback(
    (key: string, message: string) => t(key).replace('{message}', message),
    [t]
  );

  const [loading, setLoading] = useState(true);
  const [coreError, setCoreError] = useState<string | null>(null);

  const [coreJobs, setCoreJobs] = useState<CoreCronJob[]>([]);
  const [coreRunsByJob, setCoreRunsByJob] = useState<Record<string, CoreCronRun[]>>({});
  const [coreBusyKey, setCoreBusyKey] = useState<string | null>(null);

  // Create / edit modal state
  const [formOpen, setFormOpen] = useState(false);
  const [editingJob, setEditingJob] = useState<CoreCronJob | null>(null);

  const loadCoreCronJobs = useCallback(async () => {
    const response = await openhumanCronList();
    const sorted = [...response.result].sort((a, b) => {
      const aTs = new Date(a.next_run).getTime();
      const bTs = new Date(b.next_run).getTime();
      return aTs - bTs;
    });
    setCoreJobs(sorted);
  }, []);

  const loadCoreCronJobsOnly = useCallback(async () => {
    loadCronJobsLog('start');
    setLoading(true);
    setCoreError(null);

    try {
      await loadCoreCronJobs();
      loadCronJobsLog('success');
    } catch (err) {
      loadCronJobsLog('failure', err);
      const message = err instanceof Error ? err.message : String(err);
      setCoreError(formatCronError('settings.cron.jobs.errorLoadList', message));
    } finally {
      setLoading(false);
    }
  }, [formatCronError, loadCoreCronJobs]);

  useEffect(() => {
    void loadCoreCronJobsOnly();
  }, [loadCoreCronJobsOnly]);

  const toggleCoreJob = async (job: CoreCronJob) => {
    const key = `core-toggle:${job.id}`;
    setCoreBusyKey(key);
    setCoreError(null);
    try {
      const response = await openhumanCronUpdate(job.id, { enabled: !job.enabled });
      const updated = response.result;
      setCoreJobs(prev => prev.map(item => (item.id === updated.id ? updated : item)));
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setCoreError(formatCronError('settings.cron.jobs.errorToggle', message));
    } finally {
      setCoreBusyKey(null);
    }
  };

  const runCoreJob = async (jobId: string) => {
    const key = `core-run:${jobId}`;
    setCoreBusyKey(key);
    setCoreError(null);

    try {
      await openhumanCronRun(jobId);
      const runs = await openhumanCronRuns(jobId, 10);
      setCoreRunsByJob(prev => ({ ...prev, [jobId]: runs.result }));
      await loadCoreCronJobs();
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setCoreError(formatCronError('settings.cron.jobs.errorRun', message));
    } finally {
      setCoreBusyKey(null);
    }
  };

  const loadCoreRuns = async (jobId: string) => {
    const key = `core-runs:${jobId}`;
    setCoreBusyKey(key);
    setCoreError(null);

    try {
      const runs = await openhumanCronRuns(jobId, 10);
      setCoreRunsByJob(prev => ({ ...prev, [jobId]: runs.result }));
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setCoreError(formatCronError('settings.cron.jobs.errorLoadRuns', message));
    } finally {
      setCoreBusyKey(null);
    }
  };

  const handleCreate = async (params: CronAddParams) => {
    loadCronJobsLog('handleCreate metadata=%o', {
      jobType: params.job_type,
      scheduleKind: params.schedule.kind,
      hasName: Boolean(params.name),
      hasSessionTarget: Boolean(params.session_target),
      deleteAfterRun: params.delete_after_run,
    });
    try {
      await openhumanCronAdd(params);
      await loadCoreCronJobs();
      setFormOpen(false);
      loadCronJobsLog('handleCreate success');
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      loadCronJobsLog('handleCreate error: %s', message);
      setCoreError(formatCronError('settings.cron.jobs.errorCreate', message));
      throw err; // Re-throw so modal can surface inline error
    }
  };

  const handleUpdate = async (jobId: string, patch: Record<string, unknown>) => {
    const patchSchedule = patch.schedule as { kind?: string } | undefined;
    loadCronJobsLog('handleUpdate metadata=%o', {
      jobId,
      scheduleKind: patchSchedule?.kind ?? 'unknown',
      hasName: patch.name !== null,
      hasSessionTarget: 'session_target' in patch,
      deleteAfterRun: patch.delete_after_run,
    });
    try {
      await openhumanCronUpdate(jobId, patch);
      await loadCoreCronJobs();
      setEditingJob(null);
      loadCronJobsLog('handleUpdate success');
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      loadCronJobsLog('handleUpdate error: %s', message);
      setCoreError(formatCronError('settings.cron.jobs.errorUpdate', message));
      throw err; // Re-throw so modal can surface inline error
    }
  };

  const removeCoreJob = async (jobId: string) => {
    const key = `core-remove:${jobId}`;
    setCoreBusyKey(key);
    setCoreError(null);

    try {
      await openhumanCronRemove(jobId);
      setCoreJobs(prev => prev.filter(job => job.id !== jobId));
      setCoreRunsByJob(prev => {
        const next = { ...prev };
        delete next[jobId];
        return next;
      });
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setCoreError(formatCronError('settings.cron.jobs.errorRemove', message));
    } finally {
      setCoreBusyKey(null);
    }
  };

  return (
    <SettingsPanel testId="cron-jobs-panel" description={t('settings.developerMenu.cronJobs.desc')}>
      <SettingsSection title={t('cron.scheduledJobs')} description={t('cron.manageCronJobs')}>
        <div className="px-4 pb-4 space-y-4">
          <div className="pt-2">
            <Button
              type="button"
              variant="primary"
              size="sm"
              data-testid="cron-new-job"
              onClick={() => {
                setEditingJob(null);
                setFormOpen(true);
              }}>
              {t('settings.cron.jobs.createJob')}
            </Button>
          </div>

          <SettingsStatusLine saving={false} error={coreError} savingLabel="" />

          <CoreJobList
            loading={loading}
            coreJobs={coreJobs}
            coreRunsByJob={coreRunsByJob}
            coreBusyKey={coreBusyKey}
            onToggleCoreJob={job => void toggleCoreJob(job)}
            onRunCoreJob={jobId => void runCoreJob(jobId)}
            onLoadCoreRuns={jobId => void loadCoreRuns(jobId)}
            onRemoveCoreJob={jobId => void removeCoreJob(jobId)}
            onEditCoreJob={job => setEditingJob(job)}
          />

          <div>
            <Button
              type="button"
              variant="secondary"
              size="sm"
              data-testid="cron-refresh"
              onClick={() => void loadCoreCronJobsOnly()}>
              {t('cron.refreshCronJobs')}
            </Button>
          </div>
        </div>
      </SettingsSection>

      {/* Create modal */}
      {formOpen && editingJob === null && (
        <CronJobFormModal
          key="cron-form-create"
          mode="create"
          open={true}
          onClose={() => setFormOpen(false)}
          onCreate={params => handleCreate(params)}
          onUpdate={handleUpdate}
        />
      )}

      {/* Edit modal */}
      {editingJob !== null && (
        <CronJobFormModal
          key={`cron-form-edit-${editingJob.id}`}
          mode="edit"
          job={editingJob}
          open={true}
          onClose={() => setEditingJob(null)}
          onCreate={handleCreate}
          onUpdate={handleUpdate}
        />
      )}
    </SettingsPanel>
  );
};

export default CronJobsPanel;
