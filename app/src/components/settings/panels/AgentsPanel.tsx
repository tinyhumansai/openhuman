/**
 * AgentsPanel — Settings > Agents.
 *
 * Surfaces the user-facing agent registry (`openhuman.agent_registry_*`):
 * shipped built-in agents plus user-authored custom agents. Users can
 * enable/disable agents, create custom agents, edit any agent (editing a
 * built-in saves an override), and delete a custom agent / reset a built-in
 * override.
 */
import { type ReactNode, useCallback, useEffect, useRef, useState } from 'react';
import { LuPencil, LuPlus, LuRotateCcw, LuTrash2 } from 'react-icons/lu';

import { useT } from '../../../lib/i18n/I18nContext';
import {
  agentRegistryApi,
  type AgentRegistryEntry,
  type UpdateAgentInput,
} from '../../../services/api/agentRegistryApi';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const ORCHESTRATOR_ID = 'orchestrator';

function slugify(name: string): string {
  return name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, '-')
    .replace(/^-+|-+$/g, '');
}

function splitLines(value: string): string[] {
  return value
    .split('\n')
    .map(line => line.trim())
    .filter(Boolean);
}

const AgentsPanel = () => {
  const { t } = useT();
  const { navigateBack } = useSettingsNavigation();

  const [agents, setAgents] = useState<AgentRegistryEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [editing, setEditing] = useState<AgentRegistryEntry | null>(null);
  const [creating, setCreating] = useState(false);
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

  const handleSaved = useCallback((saved: AgentRegistryEntry) => {
    setAgents(prev => {
      const exists = prev.some(a => a.id === saved.id);
      return exists ? prev.map(a => (a.id === saved.id ? saved : a)) : [...prev, saved];
    });
    setEditing(null);
    setCreating(false);
  }, []);

  return (
    <div className="mx-auto max-w-3xl px-4 py-6">
      <SettingsHeader title={t('settings.agents.title')} onBack={navigateBack} />

      <div className="mb-4 flex items-start justify-between gap-3">
        <p className="text-sm text-stone-500 dark:text-neutral-400">
          {t('settings.agents.subtitle')}
        </p>
        <button
          type="button"
          onClick={() => setCreating(true)}
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
        <ul className="space-y-2">
          {agents.map(agent => (
            <AgentRow
              key={agent.id}
              agent={agent}
              busy={busyId === agent.id}
              onToggle={() => handleToggle(agent)}
              onEdit={() => setEditing(agent)}
              onRemove={() => handleRemove(agent)}
            />
          ))}
        </ul>
      )}

      {(editing || creating) && (
        <AgentEditor
          agent={editing}
          onClose={() => {
            setEditing(null);
            setCreating(false);
          }}
          onSaved={handleSaved}
        />
      )}
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
    <li
      className={`rounded-xl border border-stone-200 bg-white px-4 py-3 shadow-sm dark:border-neutral-800 dark:bg-neutral-900 ${
        agent.enabled ? '' : 'opacity-70'
      }`}>
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
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
        </div>

        <div className="flex flex-none items-center gap-2">
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
              className={`absolute top-0.5 h-4 w-4 rounded-full bg-white transition-transform ${
                agent.enabled ? 'translate-x-4' : 'translate-x-0.5'
              }`}
            />
          </button>
        </div>
      </div>

      <div className="mt-2 flex items-center justify-end gap-1">
        <button
          type="button"
          onClick={onEdit}
          className="inline-flex items-center gap-1 rounded-md px-2 py-1 text-[11px] font-medium text-stone-600 hover:bg-stone-100 dark:text-neutral-300 dark:hover:bg-neutral-800">
          <LuPencil className="h-3 w-3" />
          {t('settings.agents.edit')}
        </button>
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

function AgentEditor({
  agent,
  onClose,
  onSaved,
}: {
  agent: AgentRegistryEntry | null;
  onClose: () => void;
  onSaved: (saved: AgentRegistryEntry) => void;
}) {
  const { t } = useT();
  const isCreate = agent === null;
  const isCustom = agent?.source === 'custom';

  const [id, setId] = useState(agent?.id ?? '');
  const [idTouched, setIdTouched] = useState(!isCreate);
  const [name, setName] = useState(agent?.name ?? '');
  const [description, setDescription] = useState(agent?.description ?? '');
  const [model, setModel] = useState(agent?.model ?? '');
  const [systemPrompt, setSystemPrompt] = useState(agent?.system_prompt ?? '');
  const [tools, setTools] = useState((agent?.tool_allowlist ?? []).join('\n'));
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Auto-derive id from name while creating, until the user edits it.
  const handleName = (value: string) => {
    setName(value);
    if (isCreate && !idTouched) setId(slugify(value));
  };

  const canSubmit = name.trim().length > 0 && description.trim().length > 0 && !submitting;

  const handleSubmit = async () => {
    if (!canSubmit) return;
    setSubmitting(true);
    setError(null);
    try {
      const toolAllowlist = splitLines(tools);
      let saved: AgentRegistryEntry;
      if (isCreate) {
        saved = await agentRegistryApi.createCustom({
          id: id.trim() || slugify(name),
          name: name.trim(),
          description: description.trim(),
          model: model.trim() || null,
          system_prompt: systemPrompt.trim() || null,
          tool_allowlist: toolAllowlist,
        });
      } else {
        const patch: UpdateAgentInput = {
          name: name.trim(),
          description: description.trim(),
          model: model.trim() || null,
          system_prompt: systemPrompt.trim() || null,
          tool_allowlist: toolAllowlist,
        };
        saved = await agentRegistryApi.update(agent.id, patch);
      }
      onSaved(saved);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/30 px-4 py-6">
      <section className="max-h-full w-full max-w-lg overflow-y-auto rounded-lg border border-stone-200 bg-white p-4 shadow-xl dark:border-neutral-800 dark:bg-neutral-900">
        <h3 className="mb-3 text-base font-semibold text-stone-900 dark:text-neutral-50">
          {isCreate
            ? t('settings.agents.editor.createTitle')
            : t('settings.agents.editor.editTitle')}
        </h3>

        <div className="space-y-3 text-sm">
          <Field label={t('settings.agents.editor.name')}>
            <input
              autoFocus
              value={name}
              onChange={e => handleName(e.target.value)}
              className={inputClass}
            />
          </Field>

          {isCreate && (
            <Field label={t('settings.agents.editor.id')} hint={t('settings.agents.editor.idHint')}>
              <input
                value={id}
                onChange={e => {
                  setIdTouched(true);
                  setId(e.target.value);
                }}
                className={`${inputClass} font-mono`}
              />
            </Field>
          )}

          <Field label={t('settings.agents.editor.description')}>
            <input
              value={description}
              onChange={e => setDescription(e.target.value)}
              className={inputClass}
            />
          </Field>

          <Field label={t('settings.agents.editor.model')}>
            <input
              value={model ?? ''}
              onChange={e => setModel(e.target.value)}
              placeholder={t('settings.agents.editor.modelPlaceholder')}
              className={inputClass}
            />
          </Field>

          <Field label={t('settings.agents.editor.systemPrompt')}>
            <textarea
              value={systemPrompt ?? ''}
              onChange={e => setSystemPrompt(e.target.value)}
              rows={3}
              className={`${inputClass} resize-y`}
            />
          </Field>

          <Field
            label={t('settings.agents.editor.tools')}
            hint={t('settings.agents.editor.toolsHint')}>
            <textarea
              value={tools}
              onChange={e => setTools(e.target.value)}
              rows={3}
              className={`${inputClass} resize-y font-mono`}
            />
          </Field>

          {!isCreate && !isCustom && (
            <p className="text-[11px] text-stone-400 dark:text-neutral-500">
              {t('settings.agents.editor.defaultsNote')}
            </p>
          )}

          {error && (
            <p className="rounded-md border border-coral-200 bg-coral-50 px-3 py-2 text-xs text-coral-700 dark:border-coral-500/30 dark:bg-coral-500/10 dark:text-coral-300">
              {error}
            </p>
          )}

          <div className="flex justify-end gap-2 pt-1">
            <button
              type="button"
              onClick={onClose}
              className="rounded-md border border-stone-200 px-3 py-1.5 text-xs font-medium text-stone-600 hover:bg-stone-50 dark:border-neutral-700 dark:text-neutral-300 dark:hover:bg-neutral-800">
              {t('common.cancel')}
            </button>
            <button
              type="button"
              onClick={handleSubmit}
              disabled={!canSubmit}
              className="rounded-md bg-ocean-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-ocean-700 disabled:opacity-50">
              {submitting
                ? t('settings.agents.editor.saving')
                : isCreate
                  ? t('settings.agents.editor.create')
                  : t('settings.agents.editor.save')}
            </button>
          </div>
        </div>
      </section>
    </div>
  );
}

const inputClass =
  'w-full rounded-md border border-stone-200 bg-white px-2 py-1.5 text-sm text-stone-900 dark:border-neutral-700 dark:bg-neutral-950 dark:text-neutral-50';

function Field({ label, hint, children }: { label: string; hint?: string; children: ReactNode }) {
  return (
    <label className="block">
      <span className="mb-1 block text-xs font-semibold text-stone-500 dark:text-neutral-400">
        {label}
      </span>
      {children}
      {hint && (
        <span className="mt-1 block text-[11px] text-stone-400 dark:text-neutral-500">{hint}</span>
      )}
    </label>
  );
}

export default AgentsPanel;
