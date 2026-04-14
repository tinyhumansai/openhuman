import type { Dispatch, FormEvent, SetStateAction } from 'react';
import { useNavigate } from 'react-router-dom';

import type {
  SubconsciousEscalation,
  SubconsciousLogEntry,
  SubconsciousStatus,
  SubconsciousTask,
} from '../../utils/tauriCommands/subconscious';

const SKILL_KEYWORDS =
  /\bskill\b|\boauth\b|\bnotion\b|\bgmail\b|\bintegration\b|\bdisconnect|\breconnect|\bre-?auth/i;

function isSkillRelated(title: string, description: string): boolean {
  return SKILL_KEYWORDS.test(title) || SKILL_KEYWORDS.test(description);
}

interface IntelligenceSubconsciousTabProps {
  addSubconsciousTask: (title: string) => Promise<void>;
  approveEscalation: (escalationId: string) => Promise<void>;
  dismissEscalation: (escalationId: string) => Promise<void>;
  expandedLogIds: Set<string>;
  logEntries: SubconsciousLogEntry[];
  newTaskTitle: string;
  removeSubconsciousTask: (taskId: string) => Promise<void>;
  setExpandedLogIds: Dispatch<SetStateAction<Set<string>>>;
  setNewTaskTitle: (value: string) => void;
  status: SubconsciousStatus | null;
  tasks: SubconsciousTask[];
  toggleSubconsciousTask: (taskId: string, enabled: boolean) => Promise<void>;
  triggerTick: () => Promise<void>;
  triggering: boolean;
  escalations: SubconsciousEscalation[];
  loading: boolean;
}

export default function IntelligenceSubconsciousTab({
  addSubconsciousTask,
  approveEscalation,
  dismissEscalation,
  escalations,
  expandedLogIds,
  loading,
  logEntries,
  newTaskTitle,
  removeSubconsciousTask,
  setExpandedLogIds,
  setNewTaskTitle,
  status,
  tasks,
  toggleSubconsciousTask,
  triggerTick,
  triggering,
}: IntelligenceSubconsciousTabProps) {
  const navigate = useNavigate();

  const handleAddTask = async (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    const title = newTaskTitle.trim();
    if (!title) return;
    console.debug('[subconscious-ui] add task:start', { title });
    try {
      await addSubconsciousTask(title);
      setNewTaskTitle('');
      console.debug('[subconscious-ui] add task:success', { title });
    } catch (error) {
      console.debug('[subconscious-ui] add task:error', {
        title,
        error: error instanceof Error ? error.message : String(error),
      });
    }
  };

  const handleRunTick = async () => {
    console.debug('[subconscious-ui] run tick:start', { triggering });
    try {
      await triggerTick();
      console.debug('[subconscious-ui] run tick:done');
    } catch (error) {
      console.debug('[subconscious-ui] run tick:error', {
        error: error instanceof Error ? error.message : String(error),
      });
    }
  };

  const handleApproveEscalation = async (escalationId: string) => {
    console.debug('[subconscious-ui] escalation approve:start', { escalationId });
    try {
      await approveEscalation(escalationId);
      console.debug('[subconscious-ui] escalation approve:success', { escalationId });
    } catch (error) {
      console.debug('[subconscious-ui] escalation approve:error', {
        escalationId,
        error: error instanceof Error ? error.message : String(error),
      });
    }
  };

  const handleDismissEscalation = async (escalationId: string) => {
    console.debug('[subconscious-ui] escalation dismiss:start', { escalationId });
    try {
      await dismissEscalation(escalationId);
      console.debug('[subconscious-ui] escalation dismiss:success', { escalationId });
    } catch (error) {
      console.debug('[subconscious-ui] escalation dismiss:error', {
        escalationId,
        error: error instanceof Error ? error.message : String(error),
      });
    }
  };

  const handleFixInSkills = (escalationId: string) => {
    console.debug('[subconscious-ui] escalation fix in skills:navigate', { escalationId });
    navigate('/skills', {
      state: { subconsciousEscalationId: escalationId },
    });
  };

  const handleToggleTask = async (taskId: string, enabled: boolean, title: string) => {
    console.debug('[subconscious-ui] task toggle:start', { taskId, enabled, title });
    try {
      await toggleSubconsciousTask(taskId, enabled);
      console.debug('[subconscious-ui] task toggle:success', { taskId, enabled, title });
    } catch (error) {
      console.debug('[subconscious-ui] task toggle:error', {
        taskId,
        enabled,
        title,
        error: error instanceof Error ? error.message : String(error),
      });
    }
  };

  const handleRemoveTask = async (taskId: string, title: string) => {
    console.debug('[subconscious-ui] task remove:start', { taskId, title });
    try {
      await removeSubconsciousTask(taskId);
      console.debug('[subconscious-ui] task remove:success', { taskId, title });
    } catch (error) {
      console.debug('[subconscious-ui] task remove:error', {
        taskId,
        title,
        error: error instanceof Error ? error.message : String(error),
      });
    }
  };

  return (
    <div className="space-y-6 animate-fade-up">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2 text-xs text-stone-400">
          {status && (
            <>
              <span>{status.task_count} tasks</span>
              <span className="text-stone-300">|</span>
              <span>{status.total_ticks} ticks</span>
              {status.last_tick_at && (
                <>
                  <span className="text-stone-300">|</span>
                  <span>Last: {new Date(status.last_tick_at * 1000).toLocaleTimeString()}</span>
                </>
              )}
              {status.consecutive_failures > 0 && (
                <>
                  <span className="text-stone-300">|</span>
                  <span className="text-coral-500">{status.consecutive_failures} failed</span>
                </>
              )}
            </>
          )}
        </div>
        <div className="flex items-center gap-2">
          <div className="flex items-center gap-1.5">
            <svg
              className="w-3 h-3 text-stone-400"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z"
              />
            </svg>
            <select
              value={status?.interval_minutes ?? 5}
              onChange={() => {
                // Config update would require restart — show as read-only for now
              }}
              disabled
              title="Tick interval (change in Settings > Advanced)"
              className="text-xs bg-stone-50 border border-stone-200 rounded px-1.5 py-0.5 text-stone-500 cursor-not-allowed">
              <option value={5}>5 min</option>
              <option value={10}>10 min</option>
              <option value={15}>15 min</option>
              <option value={30}>30 min</option>
              <option value={60}>1 hour</option>
              <option value={360}>6 hours</option>
              <option value={720}>12 hours</option>
              <option value={1440}>1 day</option>
            </select>
          </div>
          <button
            onClick={() => void handleRunTick()}
            disabled={triggering}
            className="flex items-center gap-1.5 px-3 py-1.5 text-xs bg-stone-50 hover:bg-stone-100 disabled:opacity-40 border border-stone-200 rounded-lg text-stone-600 transition-colors">
            {triggering ? (
              <div className="w-3 h-3 border border-stone-400 border-t-transparent rounded-full animate-spin" />
            ) : (
              <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M13 10V3L4 14h7v7l9-11h-7z"
                />
              </svg>
            )}
            Run Now
          </button>
        </div>
      </div>

      {escalations.length > 0 && (
        <div>
          <h3 className="text-sm font-semibold text-stone-900 mb-3 flex items-center gap-2">
            <span className="w-2 h-2 rounded-full bg-amber-400 animate-pulse" />
            Approval Needed
            <span className="text-[10px] px-1.5 py-0.5 rounded-full bg-amber-100 text-amber-700">
              {escalations.length}
            </span>
          </h3>
          <div className="space-y-2">
            {escalations.map(esc => (
              <div key={esc.id} className="bg-amber-50 border border-amber-200 rounded-xl p-4">
                <div className="flex items-start justify-between">
                  <div className="flex-1">
                    <p className="text-sm font-medium text-stone-900">{esc.title}</p>
                    <p className="text-xs text-stone-500 mt-1">{esc.description}</p>
                    <div className="flex items-center gap-2 mt-2">
                      <span
                        className={`text-[10px] px-2 py-0.5 rounded-full ${
                          esc.priority === 'critical'
                            ? 'bg-coral-100 text-coral-700'
                            : esc.priority === 'important'
                              ? 'bg-amber-100 text-amber-700'
                              : 'bg-stone-100 text-stone-600'
                        }`}>
                        {esc.priority}
                      </span>
                      <span className="text-[10px] text-stone-400">
                        Requires your approval to proceed
                      </span>
                    </div>
                  </div>
                  <div className="flex gap-2 ml-3 flex-shrink-0">
                    {isSkillRelated(esc.title, esc.description) ? (
                      <button
                        onClick={() => handleFixInSkills(esc.id)}
                        className="px-3 py-1.5 text-xs bg-primary-500 hover:bg-primary-600 text-white rounded-lg transition-colors">
                        Fix in Skills
                      </button>
                    ) : (
                      <button
                        onClick={() => void handleApproveEscalation(esc.id)}
                        className="px-3 py-1.5 text-xs bg-sage-500 hover:bg-sage-600 text-white rounded-lg transition-colors">
                        Go ahead
                      </button>
                    )}
                    <button
                      onClick={() => void handleDismissEscalation(esc.id)}
                      className="px-3 py-1.5 text-xs bg-stone-100 hover:bg-stone-200 text-stone-600 rounded-lg transition-colors">
                      Skip
                    </button>
                  </div>
                </div>
              </div>
            ))}
          </div>
        </div>
      )}

      <div>
        <h3 className="text-sm font-semibold text-stone-900 mb-3">Active Tasks</h3>
        {loading && tasks.length === 0 ? (
          <div className="text-center py-4">
            <div className="w-6 h-6 mx-auto border-2 border-stone-300 border-t-transparent rounded-full animate-spin" />
          </div>
        ) : tasks.filter(t => !t.completed).length === 0 ? (
          <p className="text-xs text-stone-400 py-3">No active tasks. Add one below.</p>
        ) : (
          <div className="space-y-1.5">
            {tasks
              .filter(t => !t.completed && t.source === 'system')
              .map(task => (
                <div key={task.id} className="flex items-center py-2 px-3 bg-stone-50 rounded-lg">
                  <div className="w-1.5 h-1.5 rounded-full bg-sage-400 flex-shrink-0 mr-2.5" />
                  <span className="text-sm text-stone-900 truncate flex-1">{task.title}</span>
                  <span className="text-[10px] text-stone-400 flex-shrink-0 px-1.5 py-0.5 rounded bg-stone-100">
                    default
                  </span>
                </div>
              ))}
            {tasks
              .filter(t => !t.completed && t.source !== 'system')
              .map(task => (
                <div
                  key={task.id}
                  className="flex items-center justify-between py-2 px-3 bg-stone-50 rounded-lg group">
                  <div className="flex items-center gap-2.5 flex-1 min-w-0">
                    <button
                      type="button"
                      aria-pressed={task.enabled}
                      aria-label={`${task.enabled ? 'Disable' : 'Enable'} ${task.title}`}
                      onClick={() => void handleToggleTask(task.id, !task.enabled, task.title)}
                      className={`relative w-7 h-4 rounded-full flex-shrink-0 transition-colors ${
                        task.enabled ? 'bg-sage-500' : 'bg-stone-300'
                      }`}>
                      <span
                        className={`absolute top-0.5 left-0.5 w-3 h-3 rounded-full bg-white shadow transition-transform ${
                          task.enabled ? 'translate-x-3' : 'translate-x-0'
                        }`}
                      />
                    </button>
                    <span
                      className={`text-sm truncate ${task.enabled ? 'text-stone-900' : 'text-stone-400'}`}>
                      {task.title}
                    </span>
                  </div>
                  <button
                    type="button"
                    aria-label={`Remove ${task.title}`}
                    onClick={() => void handleRemoveTask(task.id, task.title)}
                    className="opacity-0 group-hover:opacity-100 p-1 text-stone-400 hover:text-coral-500 transition-all">
                    <svg
                      className="w-3.5 h-3.5"
                      fill="none"
                      stroke="currentColor"
                      viewBox="0 0 24 24">
                      <path
                        strokeLinecap="round"
                        strokeLinejoin="round"
                        strokeWidth={2}
                        d="M6 18L18 6M6 6l12 12"
                      />
                    </svg>
                  </button>
                </div>
              ))}
          </div>
        )}

        <form onSubmit={e => void handleAddTask(e)} className="flex gap-2 mt-3">
          <input
            type="text"
            placeholder="Add a task... (e.g. 'Check urgent emails')"
            value={newTaskTitle}
            onChange={e => setNewTaskTitle(e.target.value)}
            className="flex-1 px-3 py-2 text-sm bg-white border border-stone-200 rounded-lg text-stone-900 placeholder-stone-400 focus:outline-none focus:border-primary-500/50 transition-colors"
          />
          <button
            type="submit"
            disabled={!newTaskTitle.trim()}
            className="px-3 py-2 text-sm bg-primary-500 hover:bg-primary-600 disabled:opacity-40 text-white rounded-lg transition-colors">
            Add
          </button>
        </form>
      </div>

      <div>
        <h3 className="text-sm font-semibold text-stone-900 mb-3">Activity Log</h3>
        {logEntries.length === 0 ? (
          <p className="text-xs text-stone-400 py-3">No activity yet. Run a tick to see results.</p>
        ) : (
          <div className="space-y-1 max-h-64 overflow-y-auto">
            {logEntries.map(entry => (
              <div key={entry.id} className="flex items-start gap-2 py-1.5 px-2 text-xs">
                <span className="text-stone-400 flex-shrink-0 w-14">
                  {new Date(entry.tick_at * 1000).toLocaleTimeString([], {
                    hour: '2-digit',
                    minute: '2-digit',
                  })}
                </span>
                <span
                  className={`flex-shrink-0 w-1.5 h-1.5 rounded-full mt-1.5 ${
                    entry.decision === 'act'
                      ? 'bg-sage-400'
                      : entry.decision === 'in_progress'
                        ? 'bg-primary-400 animate-pulse'
                        : entry.decision === 'escalate'
                          ? 'bg-amber-400'
                          : entry.decision === 'failed'
                            ? 'bg-coral-400'
                            : entry.decision === 'cancelled'
                              ? 'bg-stone-300'
                              : entry.decision === 'dismissed'
                                ? 'bg-stone-300'
                                : 'bg-stone-200'
                  }`}
                />
                <span
                  className={`break-words min-w-0 ${
                    entry.decision === 'in_progress'
                      ? 'text-stone-400'
                      : entry.decision === 'failed'
                        ? 'text-coral-500'
                        : 'text-stone-600'
                  } ${entry.result && entry.result.length > 120 ? 'cursor-pointer hover:text-stone-900' : ''}`}
                  onClick={() => {
                    if (entry.result && entry.result.length > 120) {
                      setExpandedLogIds(prev => {
                        const next = new Set(prev);
                        if (next.has(entry.id)) next.delete(entry.id);
                        else next.add(entry.id);
                        return next;
                      });
                    }
                  }}>
                  {entry.result
                    ? expandedLogIds.has(entry.id)
                      ? entry.result
                      : entry.result.length > 120
                        ? `${entry.result.substring(0, 120)}...`
                        : entry.result
                    : entry.decision === 'noop'
                      ? 'Nothing new'
                      : entry.decision === 'act'
                        ? 'Completed'
                        : entry.decision === 'in_progress'
                          ? 'Evaluating...'
                          : entry.decision === 'escalate'
                            ? 'Waiting for approval'
                            : entry.decision === 'failed'
                              ? 'Failed'
                              : entry.decision === 'cancelled'
                                ? 'Cancelled'
                                : entry.decision === 'dismissed'
                                  ? 'Skipped'
                                  : entry.decision}
                </span>
                {entry.duration_ms != null && (
                  <span className="text-stone-300 flex-shrink-0 ml-auto">
                    {entry.duration_ms > 1000
                      ? `${(entry.duration_ms / 1000).toFixed(1)}s`
                      : `${entry.duration_ms}ms`}
                  </span>
                )}
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
