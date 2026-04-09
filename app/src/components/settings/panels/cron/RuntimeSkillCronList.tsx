import type { RuntimeSkillOption } from '../../../../utils/tauriCommands';

interface CronSkillConfig {
  skillId: string;
  name: string;
  enabled: boolean;
  manifestTickInterval: number | null;
  options: RuntimeSkillOption[];
  optionsError: string | null;
}

interface RuntimeSkillCronListProps {
  loading: boolean;
  skills: CronSkillConfig[];
  draftValues: Record<string, string>;
  savingKey: string | null;
  onSetDraftValues: (updater: (prev: Record<string, string>) => Record<string, string>) => void;
  onSaveOptionValue: (skillId: string, option: RuntimeSkillOption, rawValue: string) => void;
  onToggleBooleanOption: (skillId: string, option: RuntimeSkillOption) => void;
}

const RuntimeSkillCronList = ({
  loading,
  skills,
  draftValues,
  savingKey,
  onSetDraftValues,
  onSaveOptionValue,
  onToggleBooleanOption,
}: RuntimeSkillCronListProps) => {
  const hasAnyRuntimeCronConfig = skills.length > 0;

  return (
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
                            onChange={() => onToggleBooleanOption(skill.skillId, option)}
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
                              onSetDraftValues(prev => ({ ...prev, [optionKey]: next }));
                              onSaveOptionValue(skill.skillId, option, next);
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
                            disabled={busy || !skill.enabled}
                            onChange={event =>
                              onSetDraftValues(prev => ({
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
                            onClick={() => onSaveOptionValue(skill.skillId, option, draft)}>
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
  );
};

export default RuntimeSkillCronList;
