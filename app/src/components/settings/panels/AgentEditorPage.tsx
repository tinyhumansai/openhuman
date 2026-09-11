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
import { useCallback, useEffect, useRef, useState } from 'react';
import { useLocation, useNavigate, useParams } from 'react-router-dom';

import { useT } from '../../../lib/i18n/I18nContext';
import { agentRegistryApi, type AgentRegistryEntry } from '../../../services/api/agentRegistryApi';
import { listProviderModels, type ModelInfo } from '../../../services/api/aiSettingsApi';
import { Alert, AlertDescription } from '../../ui/Alert';
import Badge from '../../ui/Badge';
import Button from '../../ui/Button';
import Card from '../../ui/Card';
import Field from '../../ui/Field';
import { CenteredLoadingState } from '../../ui/LoadingState';
import NativeSelect from '../../ui/NativeSelect';
import TextArea from '../../ui/TextArea';
import TextField from '../../ui/TextField';
import SettingsPanel from '../layout/SettingsPanel';
import { AgentEditorToolsField } from './AgentEditorToolsPicker';

// Known model options — mirrors the Rust tier constants + route hints
// (src/openhuman/config/schema/types.rs, inference/provider/router.rs).
// Empty string means "inherit" (no override). Any other value not in this list
// is treated as a raw BYOK provider model id (custom).
const MODEL_HINTS = [
  'hint:reasoning',
  'hint:chat',
  'hint:agentic',
  'hint:burst',
  'hint:coding',
  'hint:summarization',
  'hint:vision',
];
const MODEL_TIERS = [
  'reasoning-v1',
  'chat-v1',
  'reasoning-quick-v1',
  'agentic-v1',
  'burst-v1',
  'coding-v1',
  'summarization-v1',
  'vision-v1',
];
const KNOWN_MODELS = new Set([...MODEL_HINTS, ...MODEL_TIERS]);
const CUSTOM_MODEL = '__custom__';

/**
 * `cloud_providers` slug of the managed backend, used to list the models it can
 * serve. Pinning one here sets the agent's persistent default — unlike the
 * composer's picker, which only overrides the current conversation.
 *
 * The listing is the OpenRouter passthrough catalog and is empty unless the
 * backend has `OPENROUTER_PASSTHROUGH_ENABLED` on, so the optgroup simply does
 * not render against a backend without it.
 */
const MANAGED_PROVIDER_SLUG = 'openhuman';

/** `Display Name — $in/$out per 1M`, falling back to the bare id. */
const managedOptionLabel = (m: ModelInfo): string => {
  const name = m.display_name?.trim() || m.id;
  const inPrice = m.input_per_1m;
  const outPrice = m.output_per_1m;
  if (typeof inPrice !== 'number' || typeof outPrice !== 'number') return name;
  const fmt = (n: number) =>
    n === 0 ? '$0' : `$${n < 1 ? n.toFixed(3).replace(/0+$/, '') : n.toFixed(2)}`;
  return `${name} — ${fmt(inPrice)}/${fmt(outPrice)} per 1M`;
};

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
  const location = useLocation();
  const { id: routeId } = useParams<{ id: string }>();
  const backToList = useCallback(() => navigate('/settings/agents'), [navigate, location]);
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
  const [managedModels, setManagedModels] = useState<ModelInfo[]>([]);
  const [systemPrompt, setSystemPrompt] = useState('');
  const [toolAllowlist, setToolAllowlist] = useState<string[]>([]);

  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
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

  // Managed catalog for the optgroup below. A failure or an empty result is not
  // an error worth surfacing here: the backend returns an empty list when the
  // passthrough is off or the session is stale, and the rest of the editor is
  // unaffected — the optgroup just does not render.
  useEffect(() => {
    let active = true;
    void listProviderModels(MANAGED_PROVIDER_SLUG)
      .then(models => {
        if (active) setManagedModels(models);
      })
      .catch(() => {
        if (active) setManagedModels([]);
      });
    return () => {
      active = false;
    };
  }, []);

  // A saved managed-catalog id is not in KNOWN_MODELS, so the load above flips
  // the editor into free-text "custom" mode for it. The catalog arrives after
  // that load, so re-check once it does and select the entry properly instead of
  // showing a raw id in a text box. Only ever clears custom mode — a genuine
  // BYOK id the catalog does not contain keeps its text box.
  useEffect(() => {
    if (model && managedModels.some(m => m.id === model)) setCustomModelMode(false);
  }, [managedModels, model]);

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
    <SettingsPanel
      title={isCreate ? t('settings.agents.newAgent') : name || t('settings.agents.newAgent')}
      description={t('settings.agents.subtitle')}>
      {loading ? (
        <CenteredLoadingState label={t('common.loading')} className="py-12" />
      ) : loadError ? (
        <Alert variant="destructive">
          <AlertDescription>
            {t('settings.agents.loadError')}: {loadError}
          </AlertDescription>
        </Alert>
      ) : !isCreate && !isCustom ? (
        // Built-in agents can't be edited; they may only be enabled/disabled
        // or reset from the agents list.
        <div className="space-y-3">
          <Alert variant="default">
            <AlertDescription>{t('settings.agents.editor.builtInReadonly')}</AlertDescription>
          </Alert>
          <Button type="button" variant="secondary" size="sm" onClick={backToList}>
            {t('common.back')}
          </Button>
        </div>
      ) : (
        <div className="space-y-4">
          {/* Name — editable only on create; read-only identity on edit. */}
          <Card>
            {isCreate ? (
              <Field
                htmlFor="agent-name"
                label={t('settings.agents.editor.name')}
                stacked
                control={
                  <TextField
                    id="agent-name"
                    autoFocus
                    value={name}
                    onChange={e => handleName(e.target.value)}
                    aria-label={t('settings.agents.editor.name')}
                  />
                }
              />
            ) : (
              <Field
                label={t('settings.agents.editor.name')}
                control={
                  <div className="flex items-center gap-2">
                    <span className="text-sm font-semibold text-content">{name}</span>
                    <Badge variant="neutral">
                      {isCustom
                        ? t('settings.agents.sourceCustom')
                        : t('settings.agents.sourceDefault')}
                    </Badge>
                  </div>
                }
              />
            )}

            {/* ID — editable only on create. */}
            {isCreate ? (
              <Field
                htmlFor="agent-id"
                label={t('settings.agents.editor.id')}
                description={t('settings.agents.editor.idHint')}
                stacked
                control={
                  <TextField
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
              <Field
                label={t('settings.agents.editor.id')}
                control={<code className="font-mono text-xs text-content-muted">{agentId}</code>}
              />
            )}
          </Card>

          <Card>
            <Field
              htmlFor="agent-description"
              label={t('settings.agents.editor.description')}
              stacked
              control={
                <TextArea
                  id="agent-description"
                  value={description}
                  onChange={e => setDescription(e.target.value)}
                  rows={3}
                  aria-label={t('settings.agents.editor.description')}
                />
              }
            />
          </Card>

          {/* Model — dropdown of known hints/tiers + custom escape hatch. */}
          <Card>
            <Field
              htmlFor="agent-model"
              label={t('settings.agents.editor.model')}
              stacked
              control={
                <div className="space-y-2">
                  <NativeSelect
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
                    {managedModels.length > 0 && (
                      <optgroup label={t('settings.agents.editor.modelManaged')}>
                        {managedModels.map(m => (
                          <option key={m.id} value={m.id}>
                            {managedOptionLabel(m)}
                          </option>
                        ))}
                      </optgroup>
                    )}
                    <option value={CUSTOM_MODEL}>{t('settings.agents.editor.modelCustom')}</option>
                  </NativeSelect>
                  {customModelMode && (
                    <TextField
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
          </Card>

          <Card>
            <Field
              htmlFor="agent-system-prompt"
              label={t('settings.agents.editor.systemPrompt')}
              stacked
              control={
                <TextArea
                  id="agent-system-prompt"
                  value={systemPrompt}
                  onChange={e => setSystemPrompt(e.target.value)}
                  rows={4}
                  aria-label={t('settings.agents.editor.systemPrompt')}
                />
              }
            />
          </Card>

          {/* Allowed tools — chips + modal picker. */}
          <Card>
            <Field
              label={t('settings.agents.editor.tools')}
              description={t('settings.agents.editor.toolsHint')}
              stacked
              control={
                <AgentEditorToolsField toolAllowlist={toolAllowlist} onChange={setToolAllowlist} />
              }
            />
          </Card>

          {!isCreate && !isCustom && (
            <p className="text-[11px] text-content-faint">
              {t('settings.agents.editor.defaultsNote')}
            </p>
          )}

          {error && (
            <Alert variant="destructive" className="px-3 py-2 text-xs">
              <AlertDescription className="text-xs">{error}</AlertDescription>
            </Alert>
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
    </SettingsPanel>
  );
};

export default AgentEditorPage;
