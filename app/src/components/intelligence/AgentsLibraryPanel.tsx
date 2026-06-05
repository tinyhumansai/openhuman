import debug from 'debug';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { LuBot, LuClipboard, LuPlay, LuRefreshCw, LuSend } from 'react-icons/lu';

import { useT } from '../../lib/i18n/I18nContext';
import { type AgentDefinitionDisplay, agentLibraryApi } from '../../services/api/agentLibraryApi';

const log = debug('intelligence:agents-library');

interface AgentsLibraryPanelProps {
  onRunAgentTask: (agent: AgentDefinitionDisplay, prompt: string) => Promise<void>;
  runningAgentId?: string | null;
}

function modelLabel(
  agent: AgentDefinitionDisplay,
  t: (key: string, fallback?: string) => string
): string {
  if (agent.model.kind === 'inherit') return t('intelligence.agents.model.inherit', 'inherit');
  return agent.model.value ? `${agent.model.kind}:${agent.model.value}` : agent.model.kind;
}

function capabilityChips(
  agent: AgentDefinitionDisplay,
  t: (key: string, fallback?: string) => string
) {
  const chips = [t(`intelligence.agents.tier.${agent.tier}`, agent.tier)];
  chips.push(
    agent.write_capable ? t('intelligence.agents.writeCapable') : t('intelligence.agents.readOnly')
  );
  if (agent.uses_wildcard_tools) {
    chips.push(t('intelligence.agents.allTools'));
  } else if (agent.direct_tool_count > 0) {
    chips.push(
      (agent.direct_tool_count === 1
        ? t('intelligence.agents.toolCountOne')
        : t('intelligence.agents.toolCountOther')
      ).replace('{count}', String(agent.direct_tool_count))
    );
  }
  if (agent.subagent_ids.length > 0) {
    chips.push(
      (agent.subagent_ids.length === 1
        ? t('intelligence.agents.subagentCountOne')
        : t('intelligence.agents.subagentCountOther')
      ).replace('{count}', String(agent.subagent_ids.length))
    );
  }
  return chips;
}

export default function AgentsLibraryPanel({
  onRunAgentTask,
  runningAgentId,
}: AgentsLibraryPanelProps) {
  const { t } = useT();
  const [agents, setAgents] = useState<AgentDefinitionDisplay[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [copiedId, setCopiedId] = useState<string | null>(null);

  const loadAgents = useCallback(async () => {
    log('[ui-flow][agents-library] load entry');
    setLoading(true);
    setError(null);
    try {
      const definitions = await agentLibraryApi.listDefinitions();
      setAgents(definitions);
      log('[ui-flow][agents-library] load exit count=%d', definitions.length);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      log('[ui-flow][agents-library] load error=%s', msg);
      setError(msg);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    const handle = window.setTimeout(() => {
      void loadAgents();
    }, 0);
    return () => window.clearTimeout(handle);
  }, [loadAgents]);

  const visibleAgents = useMemo(
    () => agents.filter(agent => agent.can_run_as_user_facing_worker),
    [agents]
  );

  const handleCopy = useCallback(async (id: string) => {
    try {
      if (!navigator.clipboard?.writeText) return;
      await navigator.clipboard.writeText(id);
      setCopiedId(id);
      window.setTimeout(() => setCopiedId(current => (current === id ? null : current)), 1600);
    } catch (err) {
      log('[ui-flow][agents-library] copy failed id=%s error=%o', id, err);
    }
  }, []);

  const handleRun = useCallback(
    async (agent: AgentDefinitionDisplay, prompt: string) => {
      const trimmed = prompt.trim();
      if (!trimmed || runningAgentId) return;
      log('[ui-flow][agents-library] run entry agent=%s chars=%d', agent.id, trimmed.length);
      await onRunAgentTask(agent, trimmed);
      setDrafts(prev => ({ ...prev, [agent.id]: '' }));
      log('[ui-flow][agents-library] run exit agent=%s', agent.id);
    },
    [onRunAgentTask, runningAgentId]
  );

  return (
    <section className="space-y-3">
      <div className="flex flex-wrap items-start justify-between gap-2">
        <div className="min-w-0">
          <h3 className="flex items-center gap-2 truncate text-sm font-semibold text-stone-700 dark:text-neutral-200">
            <LuBot className="h-4 w-4 text-ocean-500" />
            {t('intelligence.agents.title')}
          </h3>
          <p className="mt-1 text-xs text-stone-400 dark:text-neutral-500">
            {t('intelligence.agents.subtitle')}
          </p>
        </div>
        <button
          type="button"
          onClick={() => void loadAgents()}
          className="inline-flex items-center gap-1.5 rounded-md border border-stone-200 px-2.5 py-1.5 text-xs font-medium text-stone-600 hover:bg-stone-50 dark:border-neutral-800 dark:text-neutral-300 dark:hover:bg-neutral-800">
          <LuRefreshCw className={`h-3.5 w-3.5 ${loading ? 'animate-spin' : ''}`} />
          {t('intelligence.agents.refresh')}
        </button>
      </div>

      {loading && (
        <div className="flex items-center justify-center rounded-xl border border-stone-200 py-5 text-sm text-stone-400 dark:border-neutral-800 dark:text-neutral-500">
          <div className="mr-2 h-4 w-4 animate-spin rounded-full border-2 border-ocean-500 border-t-transparent" />
          {t('intelligence.agents.loading')}
        </div>
      )}

      {error && !loading && (
        <div className="rounded-xl border border-coral-200 bg-coral-50 px-4 py-3 text-sm text-coral-700 dark:border-coral-500/30 dark:bg-coral-500/10 dark:text-coral-300">
          {t('intelligence.agents.failedToLoad')}: {error}
        </div>
      )}

      {!loading && !error && visibleAgents.length === 0 && (
        <div className="rounded-xl border border-dashed border-stone-200 py-6 text-center text-sm text-stone-400 dark:border-neutral-800 dark:text-neutral-500">
          {t('intelligence.agents.empty')}
        </div>
      )}

      {!loading && !error && visibleAgents.length > 0 && (
        <ul className="divide-y divide-stone-100 rounded-xl border border-stone-200 bg-white dark:divide-neutral-800 dark:border-neutral-800 dark:bg-neutral-900">
          {visibleAgents.map(agent => {
            const draft = drafts[agent.id] ?? '';
            const running = runningAgentId === agent.id;
            return (
              <li key={agent.id} className="space-y-3 p-3">
                <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
                  <div className="min-w-0 space-y-1">
                    <div className="flex flex-wrap items-center gap-1.5">
                      <span className="truncate text-sm font-semibold text-stone-800 dark:text-neutral-100">
                        {agent.display_name}
                      </span>
                      <span className="rounded-md bg-stone-100 px-1.5 py-0.5 font-mono text-[10px] text-stone-500 dark:bg-neutral-800 dark:text-neutral-400">
                        {agent.id}
                      </span>
                      <span className="rounded-md bg-ocean-50 px-1.5 py-0.5 text-[10px] font-medium text-ocean-700 dark:bg-ocean-500/10 dark:text-ocean-200">
                        {modelLabel(agent, t)}
                      </span>
                    </div>
                    <p className="text-xs leading-5 text-stone-500 dark:text-neutral-400">
                      {agent.when_to_use}
                    </p>
                    <div className="flex flex-wrap gap-1.5">
                      {capabilityChips(agent, t).map(chip => (
                        <span
                          key={chip}
                          className="rounded-md bg-stone-50 px-1.5 py-0.5 text-[10px] font-medium text-stone-500 dark:bg-neutral-800 dark:text-neutral-400">
                          {chip}
                        </span>
                      ))}
                    </div>
                  </div>
                  <div className="flex flex-none flex-wrap gap-1.5">
                    <button
                      type="button"
                      onClick={() =>
                        void handleRun(agent, t('intelligence.agents.startChatPrompt'))
                      }
                      disabled={Boolean(runningAgentId)}
                      className="inline-flex items-center gap-1.5 rounded-md border border-stone-200 px-2.5 py-1.5 text-xs font-medium text-stone-600 hover:bg-stone-50 disabled:opacity-50 dark:border-neutral-800 dark:text-neutral-300 dark:hover:bg-neutral-800">
                      <LuSend className="h-3.5 w-3.5" />
                      {t('intelligence.agents.startChat')}
                    </button>
                    <button
                      type="button"
                      onClick={() => void handleCopy(agent.id)}
                      className="inline-flex items-center gap-1.5 rounded-md border border-stone-200 px-2.5 py-1.5 text-xs font-medium text-stone-600 hover:bg-stone-50 dark:border-neutral-800 dark:text-neutral-300 dark:hover:bg-neutral-800">
                      <LuClipboard className="h-3.5 w-3.5" />
                      {copiedId === agent.id
                        ? t('intelligence.agents.copied')
                        : t('intelligence.agents.copyId')}
                    </button>
                  </div>
                </div>

                <div className="flex flex-col gap-2 sm:flex-row">
                  <input
                    type="text"
                    aria-label={t('intelligence.agents.taskPlaceholder')}
                    value={draft}
                    onChange={event =>
                      setDrafts(prev => ({ ...prev, [agent.id]: event.target.value }))
                    }
                    placeholder={t('intelligence.agents.taskPlaceholder')}
                    className="min-w-0 flex-1 rounded-md border border-stone-200 bg-white px-3 py-2 text-sm text-stone-800 placeholder:text-stone-400 focus:border-ocean-400 focus:outline-none focus:ring-2 focus:ring-ocean-100 dark:border-neutral-800 dark:bg-neutral-950 dark:text-neutral-100 dark:placeholder:text-neutral-600"
                  />
                  <button
                    type="button"
                    onClick={() => void handleRun(agent, draft)}
                    disabled={!draft.trim() || Boolean(runningAgentId)}
                    className="inline-flex flex-none items-center justify-center gap-1.5 rounded-md bg-ocean-600 px-3 py-2 text-sm font-medium text-white hover:bg-ocean-700 disabled:opacity-50">
                    <LuPlay className="h-4 w-4" />
                    {running ? t('intelligence.agents.running') : t('intelligence.agents.runTask')}
                  </button>
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}
