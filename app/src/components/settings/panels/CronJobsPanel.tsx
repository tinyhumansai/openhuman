import { useCallback, useEffect, useMemo, useState } from 'react';

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
  const { navigateBack } = useSettingsNavigation();

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

  const hasAnyRuntimeCronConfig = useMemo(() => skills.length > 0, [skills]);

  const updateOptionInState = (
    skillId: string,
    optionName: string,
    value: RuntimeSkillOption['value']
  ) => {
    setSkills(prev =>
      prev.map(skill => {
        if (skill.skillId !== skillId) {
          return skill;
        }
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
    <div className="h-full flex flex-col">
      <SettingsHeader title="Cron Jobs" showBackButton={true} onBack={navigateBack} />

      <div className="flex-1 overflow-y-auto px-6 pb-10 space-y-6">
        <section className="rounded-xl border border-stone-200 bg-white p-4 space-y-2">
          <h3 className="text-lg font-semibold text-stone-900">Scheduled Jobs</h3>
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

        <section className="rounded-xl border border-stone-200 bg-white">
          <div className="p-4 border-b border-stone-200">
            <h3 className="text-sm font-semibold text-stone-900">Core Cron Jobs</h3>
            <p className="text-xs text-stone-500 mt-1">
              Jobs persisted in the OpenHuman core scheduler database.
            </p>
          </div>

          {loading && <div className="p-4 text-sm text-stone-400">Loading cron jobs...</div>}

          {!loading && coreJobs.length === 0 && (
            <div className="p-4 text-sm text-stone-400">No core cron jobs found.</div>
          )}

          {!loading &&
            coreJobs.map((job, index) => {
              const runs = coreRunsByJob[job.id] ?? [];
              return (
                <div
                  key={job.id}
                  className={`p-4 ${index === 0 ? '' : 'border-t border-stone-200'} space-y-3`}>
                  <div className="flex items-start justify-between gap-3">
                    <div>
                      <div className="text-sm font-semibold text-stone-900">
                        {job.name || job.id}
                      </div>
                      <div className="text-[11px] text-stone-400">{job.id}</div>
                    </div>
                    <span
                      className={`px-2 py-1 text-[11px] font-semibold uppercase border rounded-full ${
                        job.enabled
                          ? 'bg-sage-50 text-sage-700 border-sage-200'
                          : 'bg-stone-100 text-stone-600 border-stone-200'
                      }`}>
                      {job.enabled ? 'Enabled' : 'Paused'}
                    </span>
                  </div>

                  <div className="text-xs text-stone-600 space-y-1">
                    <div>
                      Schedule:{' '}
                      <span className="font-medium text-stone-700">
                        {job.schedule.kind === 'cron'
                          ? job.schedule.expr
                          : job.schedule.kind === 'every'
                            ? `every ${job.schedule.every_ms}ms`
                            : `at ${job.schedule.at}`}
                      </span>
                    </div>
                    <div>
                      Next run:{' '}
                      <span className="font-medium text-stone-700">
                        {new Date(job.next_run).toLocaleString()}
                      </span>
                    </div>
                    {job.last_status && (
                      <div>
                        Last status:{' '}
                        <span className="font-medium text-stone-700">{job.last_status}</span>
                      </div>
                    )}
                  </div>

                  <div className="flex flex-wrap gap-2">
                    <button
                      type="button"
                      className="btn btn-sm btn-outline"
                      disabled={coreBusyKey === `core-toggle:${job.id}`}
                      onClick={() => {
                        void toggleCoreJob(job);
                      }}>
                      {coreBusyKey === `core-toggle:${job.id}`
                        ? 'Saving…'
                        : job.enabled
                          ? 'Pause'
                          : 'Resume'}
                    </button>
                    <button
                      type="button"
                      className="btn btn-sm btn-outline"
                      disabled={coreBusyKey === `core-run:${job.id}`}
                      onClick={() => {
                        void runCoreJob(job.id);
                      }}>
                      {coreBusyKey === `core-run:${job.id}` ? 'Running…' : 'Run Now'}
                    </button>
                    <button
                      type="button"
                      className="btn btn-sm btn-outline"
                      disabled={coreBusyKey === `core-runs:${job.id}`}
                      onClick={() => {
                        void loadCoreRuns(job.id);
                      }}>
                      {coreBusyKey === `core-runs:${job.id}` ? 'Loading…' : 'View Runs'}
                    </button>
                    <button
                      type="button"
                      className="btn btn-sm btn-error"
                      disabled={coreBusyKey === `core-remove:${job.id}`}
                      onClick={() => {
                        void removeCoreJob(job.id);
                      }}>
                      {coreBusyKey === `core-remove:${job.id}` ? 'Removing…' : 'Remove'}
                    </button>
                  </div>

                  {runs.length > 0 && (
                    <div className="rounded-lg border border-stone-200 bg-stone-50 p-3 space-y-1">
                      <div className="text-[11px] uppercase tracking-wide text-stone-400">
                        Recent Runs
                      </div>
                      {runs.map(run => (
                        <div key={run.id} className="text-xs text-stone-600">
                          <span className="font-medium text-stone-700">{run.status}</span> at{' '}
                          {new Date(run.finished_at).toLocaleString()}
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              );
            })}
        </section>

        <section className="rounded-xl border border-stone-200 bg-white">
          <div className="p-4 border-b border-stone-200">
            <h3 className="text-sm font-semibold text-stone-900">Runtime Skill Cron Settings</h3>
            <p className="text-xs text-stone-500 mt-1">
              Skill-level cron and interval options from the runtime.
            </p>
          </div>

          {loading && (
            <div className="p-4 text-sm text-stone-400">Loading runtime cron settings...</div>
          )}

          {!loading && !hasAnyRuntimeCronConfig && (
            <div className="p-4 text-sm text-stone-400">
              No cron-capable skills were found in the current runtime.
            </div>
          )}

          {!loading &&
            skills.map((skill, skillIndex) => (
              <div
                key={skill.skillId}
                className={`p-4 ${skillIndex === 0 ? '' : 'border-t border-stone-200'}`}>
                <div className="flex items-center justify-between gap-3 mb-3">
                  <div>
                    <div className="text-sm font-semibold text-stone-900">{skill.name}</div>
                    <div className="text-[11px] text-stone-400">{skill.skillId}</div>
                  </div>
                  <span
                    className={`px-2 py-1 text-[11px] font-semibold uppercase border rounded-full ${
                      skill.enabled
                        ? 'bg-sage-50 text-sage-700 border-sage-200'
                        : 'bg-stone-100 text-stone-600 border-stone-200'
                    }`}>
                    {skill.enabled ? 'Enabled' : 'Disabled'}
                  </span>
                </div>

                {skill.manifestTickInterval !== null && (
                  <div className="rounded-lg border border-stone-200 bg-stone-50 px-3 py-2 text-xs text-stone-600 mb-3">
                    Manifest tick interval:{' '}
                    <span className="font-semibold">{skill.manifestTickInterval}s</span>
                  </div>
                )}

                {skill.optionsError && (
                  <div className="rounded-lg border border-amber-300 bg-amber-50 px-3 py-2 text-xs text-amber-700 mb-3">
                    Could not load runtime options: {skill.optionsError}
                  </div>
                )}

                {skill.options.length > 0 && (
                  <div className="space-y-2">
                    {skill.options.map(option => {
                      const optionKey = `${skill.skillId}:${option.name}`;
                      const draft = draftValues[optionKey] ?? '';
                      const busy = savingKey === optionKey;

                      return (
                        <div
                          key={optionKey}
                          className="rounded-lg border border-stone-200 bg-stone-50 p-3 space-y-2">
                          <div>
                            <div className="text-xs font-medium text-stone-700">{option.label}</div>
                            {option.description && (
                              <div className="text-[11px] text-stone-500 mt-0.5">
                                {option.description}
                              </div>
                            )}
                          </div>

                          {option.type === 'boolean' && (
                            <label className="flex items-center gap-2 text-xs text-stone-600">
                              <input
                                type="checkbox"
                                className="checkbox checkbox-primary"
                                checked={option.value === true}
                                disabled={busy || !skill.enabled}
                                onChange={() => {
                                  void toggleBooleanOption(skill.skillId, option);
                                }}
                              />
                              {busy ? 'Saving…' : option.value === true ? 'Enabled' : 'Disabled'}
                            </label>
                          )}

                          {option.type === 'select' && option.options && (
                            <div className="flex items-center gap-2">
                              <select
                                value={draft}
                                disabled={busy || !skill.enabled}
                                onChange={event => {
                                  const next = event.target.value;
                                  setDraftValues(prev => ({ ...prev, [optionKey]: next }));
                                  void saveOptionValue(skill.skillId, option, next);
                                }}
                                className="select select-bordered w-full text-slate-900 bg-white">
                                {option.options.map(item => (
                                  <option key={item.value} value={item.value}>
                                    {item.label}
                                  </option>
                                ))}
                              </select>
                            </div>
                          )}

                          {(option.type === 'text' || option.type === 'number') && (
                            <div className="flex items-center gap-2">
                              <input
                                type={option.type === 'number' ? 'number' : 'text'}
                                value={draft}
                                disabled={busy}
                                onChange={event =>
                                  setDraftValues(prev => ({
                                    ...prev,
                                    [optionKey]: event.target.value,
                                  }))
                                }
                                className="input input-bordered w-full text-slate-900 bg-white"
                                placeholder={option.type === 'number' ? '60' : '*/5 * * * *'}
                              />
                              <button
                                type="button"
                                className="btn btn-primary btn-sm"
                                disabled={busy || !skill.enabled}
                                onClick={() => {
                                  void saveOptionValue(skill.skillId, option, draft);
                                }}>
                                {busy ? 'Saving…' : 'Save'}
                              </button>
                            </div>
                          )}
                        </div>
                      );
                    })}
                  </div>
                )}
              </div>
            ))}
        </section>

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
