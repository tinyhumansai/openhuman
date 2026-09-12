import { useEffect, useMemo, useState } from 'react';

import { cn } from '../../../../lib/cn';
import { useT } from '../../../../lib/i18n/I18nContext';
import { listProviderModels, type ModelInfo } from '../../../../services/api/aiSettingsApi';
import Alert from '../../../ui/Alert';
import Button from '../../../ui/Button';
import { ModalShell } from '../../../ui/ModalShell';
import NativeSelect from '../../../ui/NativeSelect';
import TextField from '../../../ui/TextField';
import {
  CLAUDE_CODE_DEFAULT_MODEL,
  type CloudProvider,
  type CustomDialogSource,
  type OllamaModel,
  slugTone,
} from './aiPanelTypes';
import { ModelEntryField, useModelEntryMode } from './ModelEntryField';
import { ProviderSwatch } from './ProviderListRow';

type TFn = (key: string, fallback?: string) => string;

/**
 * `cloud_providers` slug of the managed backend. The picker models managed as
 * its own `{kind:'managed'}` source (settings filters the raw `openhuman` row
 * out of `cloudProviders`), but the core still addresses it by slug when
 * listing models.
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

export interface ProviderModelSelection {
  source: CustomDialogSource;
  model: string;
  /** Provider-reported context window for this exact model, when known. */
  contextWindow?: number | null;
}

interface ProviderModelPickerDialogProps {
  /**
   * Offer "Managed by OpenHuman" as the first source. Default `true`: managed
   * is the product's own routing and must stay reachable from anywhere a model
   * is chosen, or picking a specific model becomes a one-way door.
   *
   * Pass `false` only where managed would contradict the surface itself — the
   * "Use Your Own Models" card being the one case: choosing managed inside it
   * would silently flip the routing mode out from under the card.
   */
  allowManaged?: boolean;
  cloudProviders: CloudProvider[];
  localModels: OllamaModel[];
  ollamaRunning: boolean;
  claudeCodeEnabled: boolean;
  initial: ProviderModelSelection | null;
  onClose: () => void;
  onSelect: (selection: ProviderModelSelection) => void;
}

const sourceKey = (source: CustomDialogSource) =>
  source.kind === 'cloud' ? `cloud:${source.providerSlug}` : source.kind;

/** Managed needs no model id — the product chooses one per workload. */
const isManaged = (source: CustomDialogSource | null): boolean => source?.kind === 'managed';

const sourceLabel = (source: CustomDialogSource, providers: CloudProvider[], t: TFn) =>
  source.kind === 'managed'
    ? t('settings.ai.managedSourceLabel')
    : source.kind === 'cloud'
      ? (providers.find(provider => provider.slug === source.providerSlug)?.label ??
        source.providerSlug)
      : source.kind === 'local'
        ? 'Ollama'
        : 'Claude Code';

const sourceSlug = (source: CustomDialogSource) =>
  source.kind === 'managed'
    ? 'openhuman'
    : source.kind === 'cloud'
      ? source.providerSlug
      : source.kind === 'local'
        ? 'ollama'
        : 'claude-code';

const sourceDetail = (source: CustomDialogSource, t: TFn) =>
  source.kind === 'managed'
    ? t('settings.ai.managedSourceDetail')
    : source.kind === 'cloud'
      ? 'Cloud provider'
      : source.kind === 'local'
        ? 'Local runtime'
        : 'CLI provider';

/**
 * Shared, searchable provider and model chooser. It owns discovery and
 * selection only; callers keep their own persistence, validation, and test
 * flows, so the same dialog can serve global and per-workload routing.
 */
export function ProviderModelPickerDialog({
  allowManaged = true,
  cloudProviders,
  localModels,
  ollamaRunning,
  claudeCodeEnabled,
  initial,
  onClose,
  onSelect,
}: ProviderModelPickerDialogProps) {
  const { t } = useT();
  const sources = useMemo<CustomDialogSource[]>(
    () => [
      // First, and before any configured provider: it is the default the app
      // ships with, and the one a user needs to find again after trying their
      // own key.
      ...(allowManaged ? ([{ kind: 'managed' as const }] as const) : []),
      ...cloudProviders.map(provider => ({ kind: 'cloud' as const, providerSlug: provider.slug })),
      ...(ollamaRunning && localModels.length > 0 ? ([{ kind: 'local' as const }] as const) : []),
      ...(claudeCodeEnabled ? ([{ kind: 'claude-code' as const }] as const) : []),
    ],
    [allowManaged, claudeCodeEnabled, cloudProviders, localModels.length, ollamaRunning]
  );
  const [query, setQuery] = useState('');
  const [source, setSource] = useState<CustomDialogSource | null>(
    initial?.source ?? sources[0] ?? null
  );
  const [model, setModel] = useState(initial?.model ?? '');
  const [catalog, setCatalog] = useState<ModelInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [catalogError, setCatalogError] = useState<string | null>(null);
  const [catalogRequest, setCatalogRequest] = useState(0);
  const isLocalSource = source?.kind === 'local';

  const selectedCloudProvider =
    source?.kind === 'cloud'
      ? cloudProviders.find(provider => provider.slug === source.providerSlug)
      : undefined;
  const modelEntryMode = useModelEntryMode({
    endpoint: selectedCloudProvider?.endpoint,
    model,
    catalogIds: catalog.map(candidate => candidate.id),
  });

  /**
   * Slug whose `/models` listing backs the right-hand pane, or null when the
   * pane is not remote-backed (local / claude-code).
   *
   * Derived as a plain string so the fetch effect below depends on a VALUE, not
   * on the `source` object or the `localModels` array. Callers pass those
   * inline (`localModels={[]}` in ModelQualityPill), so their identity changes
   * on every render — with them in the dependency list the effect re-ran each
   * render, which for managed meant one network fetch per render and a
   * visibly thrashing dropdown.
   */
  const fetchSlug = useMemo(() => {
    if (source?.kind === 'cloud') return source.providerSlug;
    // Managed is fetched like a cloud provider: the core resolves the hosted
    // API + session JWT for the `openhuman` slug and asks for the OpenRouter
    // passthrough catalog. An empty result is expected and fine — the backend
    // returns nothing when OPENROUTER_PASSTHROUGH_ENABLED is off — and the
    // "Automatic" default keeps managed selectable either way.
    if (source?.kind === 'managed') return MANAGED_PROVIDER_SLUG;
    return null;
  }, [source]);

  // Local models are supplied by the host, not fetched. Kept separate so a new
  // array identity re-mirrors the list without re-triggering a network call.
  useEffect(() => {
    if (isLocalSource) setCatalog(localModels);
  }, [isLocalSource, localModels]);

  useEffect(() => {
    if (!fetchSlug) {
      if (!isLocalSource) setCatalog([]);
      return;
    }
    let active = true;
    setLoading(true);
    setCatalog([]);
    setCatalogError(null);
    void listProviderModels(fetchSlug)
      .then(models => {
        if (!active) return;
        setCatalog(models);
        setLoading(false);
      })
      .catch(() => {
        if (active) {
          setLoading(false);
          setCatalogError('Could not load models from this provider.');
        }
      });
    return () => {
      active = false;
    };
  }, [catalogRequest, fetchSlug, isLocalSource]);

  const filteredSources = sources.filter(candidate =>
    sourceLabel(candidate, cloudProviders, t)
      .toLocaleLowerCase()
      .includes(query.toLocaleLowerCase())
  );
  const selectSource = (nextSource: CustomDialogSource) => {
    setSource(nextSource);
    setModel(nextSource.kind === 'claude-code' ? CLAUDE_CODE_DEFAULT_MODEL : '');
    modelEntryMode.syncToEndpoint(
      nextSource.kind === 'cloud'
        ? cloudProviders.find(provider => provider.slug === nextSource.providerSlug)?.endpoint
        : undefined
    );
  };

  return (
    <ModalShell
      title={t('settings.ai.picker.title')}
      titleId="provider-model-picker-title"
      subtitle={t('settings.ai.picker.subtitle')}
      onClose={onClose}
      maxWidthClassName="max-w-3xl"
      contentClassName="p-0"
      footer={
        <div className="flex justify-end gap-2">
          <Button type="button" variant="secondary" size="sm" onClick={onClose}>
            {t('common.cancel')}
          </Button>
          <Button
            type="button"
            variant="primary"
            size="sm"
            // Managed carries no model id, so requiring one would leave the
            // only always-available option permanently unselectable.
            disabled={!source || (!isManaged(source) && !model.trim())}
            onClick={() => {
              if (!source) return;
              if (isManaged(source)) {
                // An empty model keeps the original contract (product routes
                // per workload). A pinned catalog id is forwarded like any
                // other model so the managed backend serves that exact model.
                const pinned = model.trim();
                const pinnedEntry = pinned
                  ? catalog.find(candidate => candidate.id === pinned)
                  : undefined;
                onSelect({
                  source,
                  model: pinned,
                  contextWindow:
                    pinnedEntry && (pinnedEntry.context_window ?? 0) > 0
                      ? pinnedEntry.context_window
                      : null,
                });
                return;
              }
              const selectedModel = catalog.find(candidate => candidate.id === model.trim());
              onSelect({
                source,
                model: model.trim(),
                contextWindow:
                  selectedModel && (selectedModel.context_window ?? 0) > 0
                    ? selectedModel.context_window
                    : null,
              });
            }}>
            {t('settings.ai.picker.useThisModel')}
          </Button>
        </div>
      }>
      <div className="border-b border-line-subtle p-4">
        <TextField
          value={query}
          onChange={event => setQuery(event.target.value)}
          placeholder={t('settings.ai.picker.searchPlaceholder')}
          aria-label={t('settings.ai.picker.searchPlaceholder')}
          autoFocus
        />
      </div>
      <div className="grid min-h-80 grid-cols-1 divide-y divide-line-subtle md:grid-cols-[15rem_1fr] md:divide-x md:divide-y-0">
        <div className="p-2">
          <p className="px-2 pb-2 text-xs font-medium text-content-muted">
            {t('settings.ai.picker.providersLabel')}
          </p>
          <div className="space-y-1">
            {filteredSources.map(candidate => {
              const selected = source && sourceKey(candidate) === sourceKey(source);
              return (
                <Button
                  key={sourceKey(candidate)}
                  type="button"
                  variant="tertiary"
                  size="sm"
                  onClick={() => selectSource(candidate)}
                  className={cn(
                    'h-auto w-full justify-start gap-3 px-2.5 py-2',
                    selected && 'bg-surface-muted'
                  )}>
                  <ProviderSwatch
                    slug={sourceSlug(candidate)}
                    label={sourceLabel(candidate, cloudProviders, t)}
                    tone={slugTone(sourceSlug(candidate))}
                  />
                  {/* `flex-1` + `min-w-0` is load-bearing, not cosmetic:
                      without it this wrapper sizes to its content instead of
                      shrinking, and a long provider name overflows the column
                      into the detail pane instead of wrapping inside it.
                      `min-w-0` alone does not shrink a flex item that was never
                      told it may flex.

                      The name wraps rather than truncating — a provider the
                      user cannot fully read is not a provider they can choose
                      between. The row is `h-auto`, so it grows to fit. */}
                  <span className="flex min-w-0 flex-1 flex-col items-start gap-0.5">
                    <span className="w-full text-left text-sm font-medium break-words whitespace-normal">
                      {sourceLabel(candidate, cloudProviders, t)}
                    </span>
                    <span className="w-full text-left text-xs font-normal break-words whitespace-normal text-content-muted">
                      {sourceDetail(candidate, t)}
                    </span>
                  </span>
                </Button>
              );
            })}
          </div>
        </div>
        <div className="min-w-0 p-4">
          {isManaged(source) ? (
            // Managed offers an OPTIONAL model pick. Leaving it on "Automatic"
            // preserves the original contract (empty model id -> the product
            // routes per workload); choosing a catalog entry pins that model,
            // still billed through managed credits like any tier.
            <div data-testid="model-picker-managed-pane" className="space-y-2">
              <p className="text-sm font-medium text-content">
                {t('settings.ai.managedSourceLabel')}
              </p>
              <p className="text-xs leading-relaxed text-content-muted">
                {t('settings.ai.routing.managedDesc')}
              </p>
              {loading ? (
                <NativeSelect disabled className="mt-1 w-full cursor-wait opacity-60">
                  <option>{t('settings.ai.loadingModels', 'Loading models…')}</option>
                </NativeSelect>
              ) : catalog.length > 0 ? (
                <NativeSelect
                  aria-label={t('settings.ai.modelLabel')}
                  data-testid="model-picker-managed-select"
                  value={model}
                  onChange={event => setModel(event.target.value)}
                  className="mt-1 w-full">
                  {/* Always present, unlike ModelEntryField's empty option, so a
                      pinned model can be cleared back to automatic routing. */}
                  <option value="">
                    {t('settings.ai.picker.managedAutomatic', 'Automatic (recommended)')}
                  </option>
                  {catalog.map(candidate => (
                    <option key={candidate.id} value={candidate.id}>
                      {managedOptionLabel(candidate)}
                    </option>
                  ))}
                </NativeSelect>
              ) : null}
              {catalogError ? (
                <Alert variant="destructive" className="font-mono text-xs break-all">
                  {catalogError}
                </Alert>
              ) : null}
            </div>
          ) : source?.kind === 'cloud' ? (
            <ModelEntryField
              mode={modelEntryMode}
              model={model}
              onModelChange={setModel}
              catalog={catalog}
              catalogLoading={loading}
              catalogError={catalogError}
              onRetry={() => setCatalogRequest(request => request + 1)}
              label={t('settings.ai.modelLabel')}
              placeholder={t('settings.ai.picker.modelIdPlaceholder')}
              analyticsId="settings-ai-model-picker-manual-entry"
            />
          ) : (
            <>
              <p className="mb-2 text-xs font-medium text-content-muted">
                {t('settings.ai.modelLabel')}
              </p>
              <TextField
                value={model}
                onChange={event => setModel(event.target.value)}
                placeholder={t('settings.ai.picker.modelIdPlaceholder')}
                aria-label={t('settings.ai.modelLabel')}
                mono
              />
            </>
          )}
          {source && !isManaged(source) && source.kind !== 'cloud' ? (
            <div className="mt-3 max-h-56 space-y-1 overflow-y-auto">
              {source?.kind === 'claude-code' ? (
                <p className="text-sm text-content-muted">
                  {t('settings.ai.picker.claudeCodeHint')}
                </p>
              ) : (
                catalog.map(candidate => (
                  <Button
                    key={candidate.id}
                    type="button"
                    variant="tertiary"
                    size="sm"
                    onClick={() => setModel(candidate.id)}
                    className={cn(
                      'w-full justify-start font-mono',
                      model === candidate.id && 'bg-surface-muted'
                    )}>
                    {candidate.id}
                  </Button>
                ))
              )}
            </div>
          ) : null}
        </div>
      </div>
    </ModalShell>
  );
}

export default ProviderModelPickerDialog;
