import createDebug from 'debug';
import { useCallback, useEffect, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import {
  type CoreCronJob,
  type CoreCronRun,
  openhumanCronList,
  openhumanCronRemove,
  openhumanCronRun,
  openhumanCronRuns,
  openhumanCronUpdate,
} from '../../../utils/tauriCommands';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import CoreJobList from './cron/CoreJobList';

const loadCronJobsLog = createDebug('app:settings:CronJobsPanel:loadCronSkills');

const CronJobsPanel = () => {
  const { t } = useT();
  const { navigateBack, breadcrumbs } = useSettingsNavigation();

  const [loading, setLoading] = useState(true);
  const [coreError, setCoreError] = useState<string | null>(null);

  const [coreJobs, setCoreJobs] = useState<CoreCronJob[]>([]);
  const [coreRunsByJob, setCoreRunsByJob] = useState<Record<string, CoreCronRun[]>>({});
  const [coreBusyKey, setCoreBusyKey] = useState<string | null>(null);

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
      setCoreError(`Failed to load core cron jobs: ${message}`);
    } finally {
      setLoading(false);
    }
  }, [loadCoreCronJobs]);

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
      setCoreError(`Failed to update core cron job: ${message}`);
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
      setCoreError(`Failed to run core cron job: ${message}`);
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
      setCoreError(`Failed to load run history: ${message}`);
    } finally {
      setCoreBusyKey(null);
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
      setCoreError(`Failed to remove core cron job: ${message}`);
    } finally {
      setCoreBusyKey(null);
    }
  };

  return (
    <div>
      <SettingsHeader
        title={t('cron.title')}
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div className="p-4 space-y-4">
        <section className="space-y-1">
          <h3 className="text-sm font-semibold text-stone-900">{t('cron.scheduledJobs')}</h3>
          <p className="text-xs text-stone-400">{t('cron.manageCronJobs')}</p>
        </section>

        {coreError && (
          <div className="rounded-lg border border-amber-300 bg-amber-50 px-4 py-3 text-sm text-amber-700">
            {coreError}
          </div>
        )}

        <CoreJobList
          loading={loading}
          coreJobs={coreJobs}
          coreRunsByJob={coreRunsByJob}
          coreBusyKey={coreBusyKey}
          onToggleCoreJob={job => void toggleCoreJob(job)}
          onRunCoreJob={jobId => void runCoreJob(jobId)}
          onLoadCoreRuns={jobId => void loadCoreRuns(jobId)}
          onRemoveCoreJob={jobId => void removeCoreJob(jobId)}
        />
        <div>
          <button
            type="button"
            className="btn btn-outline btn-sm"
            onClick={() => void loadCoreCronJobsOnly()}>
            {t('cron.refreshCronJobs')}
          </button>
        </div>
      </div>
    </div>
  );
};

export default CronJobsPanel;
