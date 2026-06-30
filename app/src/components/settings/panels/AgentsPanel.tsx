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

import { useT } from '../../../lib/i18n/I18nContext';
import { agentRegistryApi, type AgentRegistryEntry } from '../../../services/api/agentRegistryApi';
import Button from '../../ui/Button';
import { SettingsBadge, SettingsEmptyState, SettingsSwitch } from '../controls';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import SettingsPanel from '../layout/SettingsPanel';

const ORCHESTRATOR_ID = 'orchestrator';

const AgentsPanel = () => {
  const { t } = useT();
  const { navigateToSettings } = useSettingsNavigation();

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
    <SettingsPanel
      description={t('settings.agents.subtitle')}
      action={
        <Button
          type="button"
          variant="primary"
          size="xs"
          onClick={() => navigateToSettings('agents/new')}>
          <LuPlus className="h-3.5 w-3.5 mr-1" />
          {t('settings.agents.newAgent')}
        </Button>
      }>
      {actionError && (
        <div className="mb-3 rounded-lg border border-coral-200 bg-coral-50 px-3 py-2 text-sm text-coral-700 dark:border-coral-500/30 dark:bg-coral-500/10 dark:text-coral-300">
          {actionError}
        </div>
      )}

      {loading ? (
        <div className="flex items-center justify-center py-12 text-content-faint">
          <div className="mr-2 h-4 w-4 animate-spin rounded-full border-2 border-ocean-500 border-t-transparent" />
          <span className="text-sm">{t('common.loading')}</span>
        </div>
      ) : error ? (
        <div className="rounded-lg border border-coral-200 bg-coral-50 px-4 py-3 text-sm text-coral-700 dark:border-coral-500/30 dark:bg-coral-500/10 dark:text-coral-300">
          {t('settings.agents.loadError')}: {error}
        </div>
      ) : agents.length === 0 ? (
        <SettingsEmptyState label={t('settings.agents.empty')} />
      ) : (
        <ul className="divide-y divide-line overflow-hidden rounded-xl border border-line dark:divide-neutral-800">
          {agents.map(agent => (
            <AgentRow
              key={agent.id}
              agent={agent}
              busy={busyId === agent.id}
              onToggle={() => handleToggle(agent)}
              onEdit={() => navigateToSettings(`agents/edit/${agent.id}`)}
              onRemove={() => handleRemove(agent)}
            />
          ))}
        </ul>
      )}
    </SettingsPanel>
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
    <li className={`bg-surface px-4 py-3 ${agent.enabled ? '' : 'opacity-70'}`}>
      <div className="flex items-center justify-between gap-3">
        <div className="flex min-w-0 flex-wrap items-center gap-2">
          <h3 className="truncate text-sm font-semibold text-content">{agent.name}</h3>
          <SettingsBadge variant={isCustom ? 'primary' : 'neutral'}>
            {isCustom ? t('settings.agents.sourceCustom') : t('settings.agents.sourceDefault')}
          </SettingsBadge>
        </div>

        <SettingsSwitch
          id={`agent-toggle-${agent.id}`}
          checked={agent.enabled}
          onCheckedChange={onToggle}
          disabled={busy || isOrchestrator}
          aria-label={agent.enabled ? t('settings.agents.disable') : t('settings.agents.enable')}
        />
      </div>

      <p className="mt-1 break-words text-xs leading-snug text-content-muted">
        {agent.description}
      </p>
      <div className="mt-1.5 flex flex-wrap gap-x-3 gap-y-1 text-[11px] text-content-faint">
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
          <Button type="button" variant="tertiary" size="xs" onClick={onEdit}>
            <LuPencil className="h-3 w-3 mr-1" />
            {t('settings.agents.edit')}
          </Button>
        )}
        <Button
          type="button"
          variant="secondary"
          tone="danger"
          size="xs"
          disabled={busy}
          onClick={onRemove}>
          {isCustom ? (
            <LuTrash2 className="h-3 w-3 mr-1" />
          ) : (
            <LuRotateCcw className="h-3 w-3 mr-1" />
          )}
          {isCustom ? t('settings.agents.delete') : t('settings.agents.reset')}
        </Button>
      </div>
    </li>
  );
}

export default AgentsPanel;
