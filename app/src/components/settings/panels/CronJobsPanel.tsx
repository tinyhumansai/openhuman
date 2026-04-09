import { useCallback, useEffect, useState } from 'react';

import {
  type CoreCronJob,
  type CoreCronRun,
  openhumanCronList,
  openhumanCronRemove,
  openhumanCronRun,
  openhumanCronRuns,
  openhumanCronUpdate,
  runtimeDiscoverSkills,
  runtimeIsSkillEnabled,
  runtimeListSkillOptions,
  runtimeListSkills,
  runtimeSetSkillOption,
  type RuntimeSkillOption,
} from '../../../utils/tauriCommands';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import CoreJobList from './cron/CoreJobList';
import RuntimeSkillCronList from './cron/RuntimeSkillCronList';

type CronSkillConfig = {
  skillId: string;
  name: string;
  enabled: boolean;
  manifestTickInterval: number | null;
  options: RuntimeSkillOption[];
  optionsError: string | null;
};

const CRON_KEYWORDS = ['cron', 'schedule', 'interval', 'tick'];

const isCronOption = (option: RuntimeSkillOption): boolean => {
  const haystack = `${option.name} ${option.label} ${option.description ?? ''}`.toLowerCase();
  return CRON_KEYWORDS.some(keyword => haystack.includes(keyword));
};

const CronJobsPanel = () => {
  const { navigateBack, breadcrumbs } = useSettingsNavigation();

  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [coreError, setCoreError] = useState<string | null>(null);

  const [skills, setSkills] = useState<CronSkillConfig[]>([]);
  const [draftValues, setDraftValues] = useState<Record<string, string>>({});
  const [savingKey, setSavingKey] = useState<string | null>(null);

  const [coreJobs, setCoreJobs] = useState<CoreCronJob[]>([]);
  const [coreRunsByJob, setCoreRunsByJob] = useState<Record<string, CoreCronRun[]>>({});
  const [coreBusyKey, setCoreBusyKey] = useState<string | null>(null);

  const loadRuntimeCronSkills = useCallback(async () => {
    const [discoveredSkills, snapshots] = await Promise.all([
      runtimeDiscoverSkills(),
      runtimeListSkills(),
    ]);

    const discoveredById = new Map(discoveredSkills.map(skill => [skill.id, skill]));
    const snapshotById = new Map(snapshots.map(snapshot => [snapshot.skill_id, snapshot]));
    const allSkillIds = Array.from(new Set([...discoveredById.keys(), ...snapshotById.keys()]));

    const enabledEntries = await Promise.all(
      allSkillIds.map(async skillId => {
        try {
          const enabled = await runtimeIsSkillEnabled(skillId);
          return [skillId, enabled] as const;
        } catch {
          return [skillId, false] as const;
        }
      })
    );

    const enabledMap = Object.fromEntries(enabledEntries);

    const rows = await Promise.all(
      allSkillIds.map(async skillId => {
        const discovered = discoveredById.get(skillId);
        const snapshot = snapshotById.get(skillId);

        const baseName =
          discovered?.name?.trim() ||
          (typeof snapshot?.name === 'string' ? snapshot.name : skillId);

        let options: RuntimeSkillOption[] = [];
        let optionsError: string | null = null;

        try {
          const allOptions = await runtimeListSkillOptions(skillId);
          options = allOptions.filter(isCronOption);
        } catch (err) {
          const message = err instanceof Error ? err.message : String(err);
          optionsError = message;
        }

        const manifestTickInterval =
          typeof discovered?.tickInterval === 'number' ? discovered.tickInterval : null;

        if (manifestTickInterval === null && options.length === 0) {
          return null;
        }

        return {
          skillId,
          name: baseName,
          enabled: enabledMap[skillId] ?? false,
          manifestTickInterval,
          options,
          optionsError,
        } satisfies CronSkillConfig;
      })
    );

    const filtered = rows
      .filter((row): row is CronSkillConfig => row !== null)
      .sort((a, b) => a.name.localeCompare(b.name));

    setSkills(filtered);
    setDraftValues(prev => {
      const next = { ...prev };
      for (const skill of filtered) {
        for (const option of skill.options) {
          const key = `${skill.skillId}:${option.name}`;
          if (next[key] === undefined) {
            next[key] = option.value == null ? '' : String(option.value);
          }
        }
      }
      return next;
    });
  }, []);

  const loadCoreCronJobs = useCallback(async () => {
    const response = await openhumanCronList();
    const sorted = [...response.result].sort((a, b) => {
      const aTs = new Date(a.next_run).getTime();
      const bTs = new Date(b.next_run).getTime();
      return aTs - bTs;
    });
    setCoreJobs(sorted);
  }, []);

  const loadCronSkills = useCallback(async () => {
    setLoading(true);
    setError(null);
    setCoreError(null);

    try {
      await loadRuntimeCronSkills();
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setError(`Failed to load runtime cron jobs: ${message}`);
    }

    try {
      await loadCoreCronJobs();
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setCoreError(`Failed to load core cron jobs: ${message}`);
    } finally {
      setLoading(false);
    }
  }, [loadCoreCronJobs, loadRuntimeCronSkills]);

  useEffect(() => {
    void loadCronSkills();
  }, [loadCronSkills]);

  const updateOptionInState = (
    skillId: string,
    optionName: string,
    value: RuntimeSkillOption['value']
  ) => {
    setSkills(prev =>
      prev.map(skill => {
        if (skill.skillId !== skillId) return skill;
        return {
          ...skill,
          options: skill.options.map(option =>
            option.name === optionName ? { ...option, value } : option
          ),
        };
      })
    );
  };

  const saveOptionValue = async (skillId: string, option: RuntimeSkillOption, rawValue: string) => {
    const key = `${skillId}:${option.name}`;
    setSavingKey(key);
    setError(null);

    try {
      let nextValue: RuntimeSkillOption['value'] = rawValue;

      if (option.type === 'number') {
        const parsed = Number(rawValue);
        if (!Number.isFinite(parsed)) {
          throw new Error('Please enter a valid number for this cron option.');
        }
        nextValue = parsed;
      }

      await runtimeSetSkillOption(skillId, option.name, nextValue);
      updateOptionInState(skillId, option.name, nextValue);
      setDraftValues(prev => ({ ...prev, [key]: String(nextValue) }));
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setError(`Failed to update ${option.label}: ${message}`);
    } finally {
      setSavingKey(null);
    }
  };

  const toggleBooleanOption = async (skillId: string, option: RuntimeSkillOption) => {
    const key = `${skillId}:${option.name}`;
    setSavingKey(key);
    setError(null);

    try {
      const nextValue = !(option.value === true);
      await runtimeSetSkillOption(skillId, option.name, nextValue);
      updateOptionInState(skillId, option.name, nextValue);
      setDraftValues(prev => ({ ...prev, [key]: String(nextValue) }));
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setError(`Failed to update ${option.label}: ${message}`);
    } finally {
      setSavingKey(null);
    }
  };

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
        title="Cron Jobs"
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div className="p-4 space-y-4">
        <section className="space-y-1">
          <h3 className="text-sm font-semibold text-stone-900">Scheduled Jobs</h3>
          <p className="text-xs text-stone-400">
            Manage cron jobs from both the core scheduler and runtime skills.
          </p>
        </section>

        {error && (
          <div className="rounded-lg border border-coral-300 bg-coral-50 px-4 py-3 text-sm text-coral-700">
            {error}
          </div>
        )}

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

        <RuntimeSkillCronList
          loading={loading}
          skills={skills}
          draftValues={draftValues}
          savingKey={savingKey}
          onSetDraftValues={setDraftValues}
          onSaveOptionValue={(skillId, option, rawValue) =>
            void saveOptionValue(skillId, option, rawValue)
          }
          onToggleBooleanOption={(skillId, option) => void toggleBooleanOption(skillId, option)}
        />

        <div>
          <button
            type="button"
            className="btn btn-outline btn-sm"
            onClick={() => void loadCronSkills()}>
            Refresh Cron Jobs
          </button>
        </div>
      </div>
    </div>
  );
};

export default CronJobsPanel;
