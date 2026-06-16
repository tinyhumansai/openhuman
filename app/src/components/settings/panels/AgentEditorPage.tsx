/**
 * AgentEditorPage — Settings > Agents > (New | Edit).
 *
 * Full-page editor for a registry agent (replaces the old in-panel modal).
 * Routes: `/settings/agents/new` (create) and `/settings/agents/edit/:id`
 * (edit a default override or a custom agent).
 *
 * Field rules:
 * - Name is the page title; it is editable only when creating. On edit it is
 *   shown read-only (the agent's identity stays stable).
 * - Description is a textarea.
 * - Model is a dropdown of known route hints / tiers, with a custom-id escape
 *   hatch for BYOK provider model ids. Empty = inherit (no override).
 * - Allowed tools open a searchable modal with chip-style selection; each tool
 *   shows its description. `["*"]` means "all tools".
 */
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { LuPlus, LuSearch, LuX } from 'react-icons/lu';
import { useNavigate, useParams } from 'react-router-dom';

import { useT } from '../../../lib/i18n/I18nContext';
import {
  agentRegistryApi,
  type AgentRegistryEntry,
  type AgentToolInfo,
} from '../../../services/api/agentRegistryApi';
import PanelPage from '../../layout/PanelPage';
import Button from '../../ui/Button';
import SettingsBackButton from '../components/SettingsBackButton';
import {
  SettingsRow,
  SettingsSection,
  SettingsSelect,
  SettingsTextArea,
  SettingsTextField,
} from '../controls';

// Known model options — mirrors the Rust tier constants + route hints
// (src/openhuman/config/schema/types.rs, inference/provider/router.rs).
// Empty string means "inherit" (no override). Any other value not in this list
// is treated as a raw BYOK provider model id (custom).
const MODEL_HINTS = [
  'hint:reasoning',
  'hint:chat',
  'hint:agentic',
  'hint:coding',
  'hint:summarization',
];
const MODEL_TIERS = [
  'reasoning-v1',
  'chat-v1',
  'reasoning-quick-v1',
  'agentic-v1',
  'coding-v1',
  'summarization-v1',
];
const KNOWN_MODELS = new Set([...MODEL_HINTS, ...MODEL_TIERS]);
const CUSTOM_MODEL = '__custom__';
const ALL_TOOLS = '*';

function slugify(name: string): string {
  return name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, '-')
    .replace(/^-+|-+$/g, '');
}

const AgentEditorPage = () => {
  const { t } = useT();
  const navigate = useNavigate();
  const { id: routeId } = useParams<{ id: string }>();
  const backToList = useCallback(() => navigate('/settings/agents'), [navigate]);
  const isCreate = !routeId;

  const [loading, setLoading] = useState(!isCreate);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [isCustom, setIsCustom] = useState(true);

  // Form state.
  const [name, setName] = useState('');
  const [agentId, setAgentId] = useState('');
  const [idTouched, setIdTouched] = useState(!isCreate);
  const [description, setDescription] = useState('');
  const [model, setModel] = useState('');
  const [customModelMode, setCustomModelMode] = useState(false);
  const [systemPrompt, setSystemPrompt] = useState('');
  const [toolAllowlist, setToolAllowlist] = useState<string[]>([]);

  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [toolsOpen, setToolsOpen] = useState(false);
  const mountedRef = useRef(true);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  useEffect(() => {
    if (isCreate || !routeId) return;
    let cancelled = false;
    const load = async () => {
      setLoading(true);
      setLoadError(null);
      try {
        const agent = await agentRegistryApi.get(routeId);
        if (cancelled) return;
        if (!agent) {
          setLoadError(t('settings.agents.editor.notFound'));
          return;
        }
        populate(agent);
      } catch (err) {
        if (!cancelled) setLoadError(err instanceof Error ? err.message : String(err));
      } finally {
        if (!cancelled) setLoading(false);
      }
    };

    const populate = (agent: AgentRegistryEntry) => {
      setIsCustom(agent.source === 'custom');
      setName(agent.name);
      setAgentId(agent.id);
      setDescription(agent.description);
      const m = agent.model ?? '';
      setModel(m);
      setCustomModelMode(m !== '' && !KNOWN_MODELS.has(m));
      setSystemPrompt(agent.system_prompt ?? '');
      setToolAllowlist(agent.tool_allowlist ?? []);
    };

    void load();
    return () => {
      cancelled = true;
    };
  }, [isCreate, routeId, t]);

  const handleName = (value: string) => {
    setName(value);
    if (isCreate && !idTouched) setAgentId(slugify(value));
  };

  const allToolsSelected = toolAllowlist.length === 1 && toolAllowlist[0] === ALL_TOOLS;

  const canSubmit =
    !submitting &&
    description.trim().length > 0 &&
    (isCreate ? name.trim().length > 0 && agentId.trim().length > 0 : true);

  const handleSubmit = async () => {
    if (!canSubmit) return;
    setSubmitting(true);
    setError(null);
    const trimmedModel = model.trim();
    try {
      let saved: AgentRegistryEntry;
      if (isCreate) {
        saved = await agentRegistryApi.createCustom({
          id: agentId.trim() || slugify(name),
          name: name.trim(),
          description: description.trim(),
          model: trimmedModel || null,
          system_prompt: systemPrompt.trim() || null,
          tool_allowlist: toolAllowlist,
        });
      } else {
        saved = await agentRegistryApi.update(routeId, {
          description: description.trim(),
          // Always send a string so "inherit" (empty) clears any prior override.
          model: trimmedModel,
          system_prompt: systemPrompt.trim() || null,
          tool_allowlist: toolAllowlist,
        });
      }
      if (mountedRef.current && saved) backToList();
    } catch (err) {
      if (mountedRef.current) setError(err instanceof Error ? err.message : String(err));
    } finally {
      if (mountedRef.current) setSubmitting(false);
    }
  };

  const selectValue = customModelMode ? CUSTOM_MODEL : model;

  const onModelSelect = (value: string) => {
    if (value === CUSTOM_MODEL) {
      setCustomModelMode(true);
      setModel('');
    } else {
      setCustomModelMode(false);
      setModel(value);
    }
  };

  return (
    <PanelPage
      className="z-10"
      contentClassName=""
      description={t('settings.agents.subtitle')}
      leading={<SettingsBackButton onBack={backToList} />}>
      <div className="p-4">
        {loading ? (
          <div className="flex items-center justify-center py-12 text-neutral-400 dark:text-neutral-500">
            <div className="mr-2 h-4 w-4 animate-spin rounded-full border-2 border-ocean-500 border-t-transparent" />
            <span className="text-sm">{t('common.loading')}</span>
          </div>
        ) : loadError ? (
          <div className="rounded-lg border border-coral-200 bg-coral-50 px-4 py-3 text-sm text-coral-700 dark:border-coral-500/30 dark:bg-coral-500/10 dark:text-coral-300">
            {t('settings.agents.loadError')}: {loadError}
          </div>
        ) : !isCreate && !isCustom ? (
          // Built-in agents can't be edited; they may only be enabled/disabled
          // or reset from the agents list.
          <div className="space-y-3">
            <div className="rounded-lg border border-neutral-200 bg-neutral-50 px-4 py-3 text-sm text-neutral-600 dark:border-neutral-800 dark:bg-neutral-900 dark:text-neutral-300">
              {t('settings.agents.editor.builtInReadonly')}
            </div>
            <Button type="button" variant="secondary" size="sm" onClick={backToList}>
              {t('common.back')}
            </Button>
          </div>
        ) : (
          <div className="space-y-4">
            {/* Name — editable only on create; read-only identity on edit. */}
            <SettingsSection>
              {isCreate ? (
                <SettingsRow
                  htmlFor="agent-name"
                  label={t('settings.agents.editor.name')}
                  stacked
                  control={
                    <SettingsTextField
                      id="agent-name"
                      autoFocus
                      value={name}
                      onChange={e => handleName(e.target.value)}
                      aria-label={t('settings.agents.editor.name')}
                    />
                  }
                />
              ) : (
                <SettingsRow
                  label={t('settings.agents.editor.name')}
                  control={
                    <div className="flex items-center gap-2">
                      <span className="text-sm font-semibold text-neutral-800 dark:text-neutral-100">
                        {name}
                      </span>
                      <span className="rounded-full bg-neutral-100 px-2 py-0.5 text-[10px] font-medium text-neutral-500 dark:bg-neutral-800 dark:text-neutral-400">
                        {isCustom
                          ? t('settings.agents.sourceCustom')
                          : t('settings.agents.sourceDefault')}
                      </span>
                    </div>
                  }
                />
              )}

              {/* ID — editable only on create. */}
              {isCreate ? (
                <SettingsRow
                  htmlFor="agent-id"
                  label={t('settings.agents.editor.id')}
                  description={t('settings.agents.editor.idHint')}
                  stacked
                  control={
                    <SettingsTextField
                      id="agent-id"
                      mono
                      value={agentId}
                      onChange={e => {
                        setIdTouched(true);
                        setAgentId(e.target.value);
                      }}
                      aria-label={t('settings.agents.editor.id')}
                    />
                  }
                />
              ) : (
                <SettingsRow
                  label={t('settings.agents.editor.id')}
                  control={
                    <code className="font-mono text-xs text-neutral-500 dark:text-neutral-400">
                      {agentId}
                    </code>
                  }
                />
              )}
            </SettingsSection>

            <SettingsSection>
              <SettingsRow
                htmlFor="agent-description"
                label={t('settings.agents.editor.description')}
                stacked
                control={
                  <SettingsTextArea
                    id="agent-description"
                    value={description}
                    onChange={e => setDescription(e.target.value)}
                    rows={3}
                    aria-label={t('settings.agents.editor.description')}
                  />
                }
              />
            </SettingsSection>

            {/* Model — dropdown of known hints/tiers + custom escape hatch. */}
            <SettingsSection>
              <SettingsRow
                htmlFor="agent-model"
                label={t('settings.agents.editor.model')}
                stacked
                control={
                  <div className="space-y-2">
                    <SettingsSelect
                      id="agent-model"
                      value={selectValue}
                      onChange={e => onModelSelect(e.target.value)}
                      aria-label={t('settings.agents.editor.model')}
                      className="w-full">
                      <option value="">{t('settings.agents.editor.modelInherit')}</option>
                      <optgroup label={t('settings.agents.editor.modelHints')}>
                        {MODEL_HINTS.map(h => (
                          <option key={h} value={h}>
                            {h}
                          </option>
                        ))}
                      </optgroup>
                      <optgroup label={t('settings.agents.editor.modelTiers')}>
                        {MODEL_TIERS.map(m => (
                          <option key={m} value={m}>
                            {m}
                          </option>
                        ))}
                      </optgroup>
                      <option value={CUSTOM_MODEL}>
                        {t('settings.agents.editor.modelCustom')}
                      </option>
                    </SettingsSelect>
                    {customModelMode && (
                      <SettingsTextField
                        mono
                        value={model}
                        onChange={e => setModel(e.target.value)}
                        placeholder={t('settings.agents.editor.modelCustomPlaceholder')}
                        aria-label={t('settings.agents.editor.modelCustomPlaceholder')}
                      />
                    )}
                  </div>
                }
              />
            </SettingsSection>

            <SettingsSection>
              <SettingsRow
                htmlFor="agent-system-prompt"
                label={t('settings.agents.editor.systemPrompt')}
                stacked
                control={
                  <SettingsTextArea
                    id="agent-system-prompt"
                    value={systemPrompt}
                    onChange={e => setSystemPrompt(e.target.value)}
                    rows={4}
                    aria-label={t('settings.agents.editor.systemPrompt')}
                  />
                }
              />
            </SettingsSection>

            {/* Allowed tools — chips + modal picker. */}
            <SettingsSection>
              <SettingsRow
                label={t('settings.agents.editor.tools')}
                description={t('settings.agents.editor.toolsHint')}
                stacked
                control={
                  <div className="rounded-md border border-neutral-200 p-2 dark:border-neutral-700">
                    <div className="flex flex-wrap items-center gap-1.5">
                      {allToolsSelected ? (
                        <span className="inline-flex items-center gap-1 rounded-full bg-ocean-50 px-2.5 py-1 text-xs font-medium text-ocean-700 dark:bg-ocean-500/10 dark:text-ocean-200">
                          {t('settings.agents.editor.toolsAllSelected')}
                        </span>
                      ) : toolAllowlist.length === 0 ? (
                        <span className="px-1 py-1 text-xs text-neutral-400 dark:text-neutral-500">
                          {t('settings.agents.editor.toolsNoneSelected')}
                        </span>
                      ) : (
                        toolAllowlist.map(tool => (
                          <span
                            key={tool}
                            className="inline-flex items-center gap-1 rounded-full bg-neutral-100 px-2.5 py-1 font-mono text-xs text-neutral-700 dark:bg-neutral-800 dark:text-neutral-200">
                            {tool}
                            <button
                              type="button"
                              aria-label={t('settings.agents.editor.removeToolAria').replace(
                                '{tool}',
                                tool
                              )}
                              onClick={() => setToolAllowlist(prev => prev.filter(x => x !== tool))}
                              className="rounded-full text-neutral-400 hover:text-coral-600 dark:text-neutral-500 dark:hover:text-coral-300">
                              <LuX className="h-3 w-3" />
                            </button>
                          </span>
                        ))
                      )}
                      <button
                        type="button"
                        aria-label={t('settings.agents.editor.selectTools')}
                        onClick={() => setToolsOpen(true)}
                        className="inline-flex items-center gap-1 rounded-full border border-dashed border-neutral-300 px-2.5 py-1 text-xs font-medium text-neutral-600 hover:border-ocean-400 hover:text-ocean-600 dark:border-neutral-700 dark:text-neutral-300 dark:hover:border-ocean-500 dark:hover:text-ocean-300">
                        <LuPlus className="h-3 w-3" />
                        {t('settings.agents.editor.selectTools')}
                      </button>
                    </div>
                  </div>
                }
              />
            </SettingsSection>

            {!isCreate && !isCustom && (
              <p className="text-[11px] text-neutral-400 dark:text-neutral-500">
                {t('settings.agents.editor.defaultsNote')}
              </p>
            )}

            {error && (
              <p className="rounded-md border border-coral-200 bg-coral-50 px-3 py-2 text-xs text-coral-700 dark:border-coral-500/30 dark:bg-coral-500/10 dark:text-coral-300">
                {error}
              </p>
            )}

            <div className="flex justify-end gap-2 pt-1">
              <Button type="button" variant="secondary" size="sm" onClick={backToList}>
                {t('common.cancel')}
              </Button>
              <Button
                type="button"
                variant="primary"
                size="sm"
                onClick={() => void handleSubmit()}
                disabled={!canSubmit}>
                {submitting
                  ? t('settings.agents.editor.saving')
                  : isCreate
                    ? t('settings.agents.editor.create')
                    : t('settings.agents.editor.save')}
              </Button>
            </div>
          </div>
        )}
      </div>

      {toolsOpen && (
        <ToolsPickerModal
          allToolsSelected={allToolsSelected}
          selected={toolAllowlist}
          onToggleAll={() => setToolAllowlist(prev => (prev[0] === ALL_TOOLS ? [] : [ALL_TOOLS]))}
          onToggleTool={tool =>
            setToolAllowlist(prev => {
              const base = prev[0] === ALL_TOOLS ? [] : prev;
              return base.includes(tool) ? base.filter(x => x !== tool) : [...base, tool];
            })
          }
          onClose={() => setToolsOpen(false)}
        />
      )}
    </PanelPage>
  );
};

function ToolsPickerModal({
  allToolsSelected,
  selected,
  onToggleAll,
  onToggleTool,
  onClose,
}: {
  allToolsSelected: boolean;
  selected: string[];
  onToggleAll: () => void;
  onToggleTool: (tool: string) => void;
  onClose: () => void;
}) {
  const { t } = useT();
  const [tools, setTools] = useState<AgentToolInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState('');

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const list = await agentRegistryApi.availableTools();
      setTools(list);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return tools;
    return tools.filter(
      tool => tool.name.toLowerCase().includes(q) || tool.description.toLowerCase().includes(q)
    );
  }, [tools, query]);

  const selectedCount = allToolsSelected ? tools.length : selected.length;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/30 px-4 py-6">
      <section className="flex max-h-full w-full max-w-lg flex-col overflow-hidden rounded-lg border border-neutral-200 bg-white shadow-xl dark:border-neutral-800 dark:bg-neutral-900">
        <div className="flex items-center justify-between border-b border-neutral-200 px-4 py-3 dark:border-neutral-800">
          <div>
            <h3 className="text-base font-semibold text-neutral-900 dark:text-neutral-50">
              {t('settings.agents.editor.toolsModalTitle')}
            </h3>
            <p className="text-xs text-neutral-400 dark:text-neutral-500">
              {t('settings.agents.editor.toolsSelectedCount').replace(
                '{count}',
                String(selectedCount)
              )}
            </p>
          </div>
          <button
            type="button"
            aria-label={t('common.close')}
            onClick={onClose}
            className="rounded-full p-1 text-neutral-400 hover:bg-neutral-100 hover:text-neutral-600 dark:text-neutral-500 dark:hover:bg-neutral-800 dark:hover:text-neutral-300">
            <LuX className="h-4 w-4" />
          </button>
        </div>

        <div className="border-b border-neutral-200 px-4 py-3 dark:border-neutral-800">
          <div className="relative">
            <LuSearch className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-neutral-400 dark:text-neutral-500" />
            <SettingsTextField
              autoFocus
              value={query}
              onChange={e => setQuery(e.target.value)}
              placeholder={t('settings.agents.editor.toolsSearchPlaceholder')}
              aria-label={t('settings.agents.editor.toolsSearchPlaceholder')}
              className="pl-8"
            />
          </div>

          <button
            type="button"
            onClick={onToggleAll}
            className={`mt-2 flex w-full items-start justify-between gap-2 rounded-md border px-3 py-2 text-left transition-colors ${
              allToolsSelected
                ? 'border-ocean-400 bg-ocean-50 dark:border-ocean-500/40 dark:bg-ocean-500/10'
                : 'border-neutral-200 hover:bg-neutral-50 dark:border-neutral-700 dark:hover:bg-neutral-800'
            }`}>
            <span>
              <span className="block text-xs font-semibold text-neutral-800 dark:text-neutral-100">
                {t('settings.agents.editor.toolsAllowAll')}
              </span>
              <span className="block text-[11px] text-neutral-400 dark:text-neutral-500">
                {t('settings.agents.editor.toolsAllowAllHint')}
              </span>
            </span>
            <Checkbox checked={allToolsSelected} />
          </button>
        </div>

        <div className="min-h-[8rem] flex-1 overflow-y-auto px-2 py-2">
          {loading ? (
            <div className="flex items-center justify-center py-10 text-neutral-400 dark:text-neutral-500">
              <div className="mr-2 h-4 w-4 animate-spin rounded-full border-2 border-ocean-500 border-t-transparent" />
              <span className="text-sm">{t('settings.agents.editor.toolsLoading')}</span>
            </div>
          ) : error ? (
            <p className="px-2 py-6 text-center text-sm text-coral-600 dark:text-coral-300">
              {t('settings.agents.editor.toolsLoadError')}: {error}
            </p>
          ) : filtered.length === 0 ? (
            <p className="px-2 py-6 text-center text-sm text-neutral-400 dark:text-neutral-500">
              {t('settings.agents.editor.toolsEmpty')}
            </p>
          ) : (
            <ul>
              {filtered.map(tool => {
                const checked = allToolsSelected || selected.includes(tool.name);
                return (
                  <li key={tool.name}>
                    <button
                      type="button"
                      disabled={allToolsSelected}
                      onClick={() => onToggleTool(tool.name)}
                      className="flex w-full items-start gap-3 rounded-md px-2 py-2 text-left hover:bg-neutral-50 disabled:cursor-not-allowed disabled:opacity-50 dark:hover:bg-neutral-800">
                      <Checkbox checked={checked} className="mt-0.5" />
                      <span className="min-w-0">
                        <span className="block font-mono text-xs font-medium text-neutral-800 dark:text-neutral-100">
                          {tool.name}
                        </span>
                        <span className="block break-words text-[11px] leading-snug text-neutral-500 dark:text-neutral-400">
                          {tool.description}
                        </span>
                      </span>
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </div>

        <div className="flex justify-end border-t border-neutral-200 px-4 py-3 dark:border-neutral-800">
          <Button type="button" variant="primary" size="sm" onClick={onClose}>
            {t('settings.agents.editor.toolsDone')}
          </Button>
        </div>
      </section>
    </div>
  );
}

function Checkbox({ checked, className = '' }: { checked: boolean; className?: string }) {
  return (
    <span
      className={`flex h-4 w-4 flex-none items-center justify-center rounded border transition-colors ${
        checked
          ? 'border-ocean-600 bg-ocean-600 text-white'
          : 'border-neutral-300 bg-white dark:border-neutral-600 dark:bg-neutral-950'
      } ${className}`}>
      {checked && (
        <svg className="h-3 w-3" viewBox="0 0 20 20" fill="currentColor">
          <path
            fillRule="evenodd"
            d="M16.7 5.3a1 1 0 010 1.4l-7.5 7.5a1 1 0 01-1.4 0L3.3 9.7a1 1 0 011.4-1.4l3.3 3.3 6.8-6.8a1 1 0 011.4 0z"
            clipRule="evenodd"
          />
        </svg>
      )}
    </span>
  );
}

export default AgentEditorPage;
