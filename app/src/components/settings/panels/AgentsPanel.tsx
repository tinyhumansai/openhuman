/**
 * AgentsPanel — Settings > Agents.
 *
 * Surfaces the user-facing agent registry (`openhuman.agent_registry_*`):
 * shipped built-in agents plus user-authored custom agents. Users can
 * enable/disable agents, create custom agents, edit any agent (editing a
 * built-in saves an override), and delete a custom agent / reset a built-in
 * override.
 */
import { useCallback, useEffect, useRef, useState } from 'react';
import { LuPencil, LuPlus, LuRotateCcw, LuTrash2 } from 'react-icons/lu';
import { useNavigate } from 'react-router-dom';

import { useT } from '../../../lib/i18n/I18nContext';
import { agentRegistryApi, type AgentRegistryEntry } from '../../../services/api/agentRegistryApi';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const ORCHESTRATOR_ID = 'orchestrator';

const AgentsPanel = () => {
  const { t } = useT();
  const navigate = useNavigate();
  const { navigateBack, breadcrumbs } = useSettingsNavigation();

  const [agents, setAgents] = useState<AgentRegistryEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  const mountedRef = useRef(true);

  const load = useCallback(async () => {
    setError(null);
    try {
      const list = await agentRegistryApi.list(true);
      if (mountedRef.current) setAgents(list);
    } catch (err) {
      if (mountedRef.current) setError(err instanceof Error ? err.message : String(err));
    } finally {
      if (mountedRef.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    mountedRef.current = true;
    void load();
    return () => {
      mountedRef.current = false;
    };
  }, [load]);

  const handleToggle = useCallback(
    async (agent: AgentRegistryEntry) => {
      if (agent.id === ORCHESTRATOR_ID) return;
      setActionError(null);
      setBusyId(agent.id);
      try {
        const updated = await agentRegistryApi.setEnabled(agent.id, !agent.enabled);
        if (mountedRef.current) {
          setAgents(prev => prev.map(a => (a.id === updated.id ? updated : a)));
        }
      } catch (err) {
        if (mountedRef.current) {
          setActionError(err instanceof Error ? err.message : t('settings.agents.actionFailed'));
        }
      } finally {
        if (mountedRef.current) setBusyId(null);
      }
    },
    [t]
  );

  const handleRemove = useCallback(
    async (agent: AgentRegistryEntry) => {
      setActionError(null);
      setBusyId(agent.id);
      try {
        await agentRegistryApi.remove(agent.id);
        await load();
      } catch (err) {
        if (mountedRef.current) {
          setActionError(err instanceof Error ? err.message : t('settings.agents.actionFailed'));
        }
      } finally {
        if (mountedRef.current) setBusyId(null);
      }
    },
    [load, t]
  );

  return (
    <div className="z-10 relative">
      <SettingsHeader
        title={t('settings.agents.title')}
        showBackButton
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div className="p-4">
        <div className="mb-4 flex items-start justify-between gap-3">
          <p className="text-sm text-stone-500 dark:text-neutral-400">
            {t('settings.agents.subtitle')}
          </p>
          <button
            type="button"
            onClick={() => navigate('/settings/agents/new')}
            className="inline-flex flex-none items-center gap-1.5 rounded-md bg-ocean-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-ocean-700">
            <LuPlus className="h-3.5 w-3.5" />
            {t('settings.agents.newAgent')}
          </button>
        </div>

        {actionError && (
          <div className="mb-3 rounded-lg border border-coral-200 bg-coral-50 px-3 py-2 text-sm text-coral-700 dark:border-coral-500/30 dark:bg-coral-500/10 dark:text-coral-300">
            {actionError}
          </div>
        )}

        {loading ? (
          <div className="flex items-center justify-center py-12 text-stone-400 dark:text-neutral-500">
            <div className="mr-2 h-4 w-4 animate-spin rounded-full border-2 border-ocean-500 border-t-transparent" />
            <span className="text-sm">{t('common.loading')}</span>
          </div>
        ) : error ? (
          <div className="rounded-lg border border-coral-200 bg-coral-50 px-4 py-3 text-sm text-coral-700 dark:border-coral-500/30 dark:bg-coral-500/10 dark:text-coral-300">
            {t('settings.agents.loadError')}: {error}
          </div>
        ) : agents.length === 0 ? (
          <p className="py-12 text-center text-sm text-stone-400 dark:text-neutral-500">
            {t('settings.agents.empty')}
          </p>
        ) : (
          <ul className="divide-y divide-stone-200 overflow-hidden rounded-xl border border-stone-200 dark:divide-neutral-800 dark:border-neutral-800">
            {agents.map(agent => (
              <AgentRow
                key={agent.id}
                agent={agent}
                busy={busyId === agent.id}
                onToggle={() => handleToggle(agent)}
                onEdit={() => navigate(`/settings/agents/edit/${agent.id}`)}
                onRemove={() => handleRemove(agent)}
              />
            ))}
          </ul>
        )}
      </div>
    </div>
  );
};

function AgentRow({
  agent,
  busy,
  onToggle,
  onEdit,
  onRemove,
}: {
  agent: AgentRegistryEntry;
  busy: boolean;
  onToggle: () => void;
  onEdit: () => void;
  onRemove: () => void;
}) {
  const { t } = useT();
  const isCustom = agent.source === 'custom';
  const isOrchestrator = agent.id === ORCHESTRATOR_ID;
  const tools = agent.tool_allowlist ?? [];
  const toolsLabel = tools.includes('*')
    ? t('settings.agents.toolsAll')
    : t('settings.agents.toolsCount').replace('{count}', String(tools.length));

  return (
    <li className={`bg-white px-4 py-3 dark:bg-neutral-900 ${agent.enabled ? '' : 'opacity-70'}`}>
      <div className="flex items-center justify-between gap-3">
        <div className="flex min-w-0 flex-wrap items-center gap-2">
          <h3 className="truncate text-sm font-semibold text-stone-800 dark:text-neutral-100">
            {agent.name}
          </h3>
          <span
            className={`rounded-full px-2 py-0.5 text-[10px] font-medium ${
              isCustom
                ? 'bg-ocean-50 text-ocean-700 dark:bg-ocean-500/10 dark:text-ocean-200'
                : 'bg-stone-100 text-stone-600 dark:bg-neutral-800 dark:text-neutral-300'
            }`}>
            {isCustom ? t('settings.agents.sourceCustom') : t('settings.agents.sourceDefault')}
          </span>
        </div>

        <button
          type="button"
          role="switch"
          aria-checked={agent.enabled}
          aria-label={agent.enabled ? t('settings.agents.disable') : t('settings.agents.enable')}
          disabled={busy || isOrchestrator}
          title={isOrchestrator ? t('settings.agents.orchestratorLocked') : undefined}
          onClick={onToggle}
          className={`relative h-5 w-9 flex-none rounded-full transition-colors disabled:opacity-40 ${
            agent.enabled ? 'bg-ocean-600' : 'bg-stone-300 dark:bg-neutral-700'
          }`}>
          <span
            className={`absolute top-0.5 h-4 w-4 rounded-full bg-white shadow transition-transform dark:bg-neutral-900 ${
              agent.enabled ? 'translate-x-4' : 'translate-x-0.5'
            }`}
          />
        </button>
      </div>

      <p className="mt-1 break-words text-xs leading-snug text-stone-500 dark:text-neutral-400">
        {agent.description}
      </p>
      <div className="mt-1.5 flex flex-wrap gap-x-3 gap-y-1 text-[11px] text-stone-400 dark:text-neutral-500">
        <code className="font-mono">{agent.id}</code>
        {agent.model && (
          <span>
            {t('settings.agents.modelLabel')}: {agent.model}
          </span>
        )}
        <span>
          {t('settings.agents.toolsLabel')}: {toolsLabel}
        </span>
      </div>

      <div className="mt-2 flex items-center justify-end gap-1">
        {/* Built-in agents can't be edited — only custom agents expose Edit.
            Built-ins keep the toggle (enable/disable) and Reset (clear override). */}
        {isCustom && (
          <button
            type="button"
            onClick={onEdit}
            className="inline-flex items-center gap-1 rounded-md px-2 py-1 text-[11px] font-medium text-stone-600 hover:bg-stone-100 dark:text-neutral-300 dark:hover:bg-neutral-800">
            <LuPencil className="h-3 w-3" />
            {t('settings.agents.edit')}
          </button>
        )}
        <button
          type="button"
          disabled={busy}
          onClick={onRemove}
          className="inline-flex items-center gap-1 rounded-md px-2 py-1 text-[11px] font-medium text-coral-600 hover:bg-coral-50 disabled:opacity-40 dark:text-coral-300 dark:hover:bg-coral-500/10">
          {isCustom ? <LuTrash2 className="h-3 w-3" /> : <LuRotateCcw className="h-3 w-3" />}
          {isCustom ? t('settings.agents.delete') : t('settings.agents.reset')}
        </button>
      </div>
    </li>
  );
}

export default AgentsPanel;
