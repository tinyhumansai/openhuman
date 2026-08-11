/*
 * The model / deployment-name field shared by both AI-panel model pickers —
 * the global "Use Your Own Models" card and the per-workload routing dialog
 * (issue #5213).
 *
 * Both pickers used to reimplement the same state machine: derive "is this an
 * Azure connection" from the endpoint host, seed manual-vs-catalog entry mode
 * from it, render either a free-text field or the probed `/models` dropdown,
 * offer an escape hatch between them, and surface the Azure help + legacy-value
 * hint. The two copies had already begun to diverge (one had a `mono` field and
 * a "select a model" placeholder option, the other did not), so the behaviour
 * lives here once.
 *
 * Why the mode exists at all: Azure AI Foundry routes inference by the user's
 * **deployment name**, while `/models` lists the **base model ids** deployments
 * are created from. A closed dropdown sourced from that catalog therefore makes
 * the only correct value unreachable. Free text is the default for Azure, and
 * an explicit toggle exposes it for every other provider too, which also
 * unblocks any provider whose listing omits a model the user is entitled to.
 */
import { useCallback, useMemo, useState } from 'react';

import { useT } from '../../../../lib/i18n/I18nContext';
import type { ModelInfo } from '../../../../services/api/aiSettingsApi';
import Button from '../../../ui/Button';
import { SettingsSelect, SettingsTextField } from '../../controls';
import { isAzureFoundryEndpoint, looksLikeAzureBaseModelId } from '../azureDeployment';

const CURSOR_PARAM_MARKER = '~p=';

type CursorModelCatalog = {
  id: string;
  parameters: Map<string, Set<string>>;
};

type CursorSelection = {
  id: string;
  parameters: Map<string, string>;
};

const CURSOR_PARAMETER_LABELS: Record<string, string> = {
  reasoning: 'Reasoning effort',
  effort: 'Reasoning effort',
  thinking: 'Thinking',
  context: 'Context window',
  fast: 'Fast mode',
};

const cursorParameterOrder = (id: string): number => {
  const order = ['reasoning', 'effort', 'thinking', 'context', 'fast'];
  const index = order.indexOf(id);
  return index < 0 ? order.length : index;
};

function parseCursorSelection(value: string): CursorSelection {
  const firstParameter = value.indexOf(CURSOR_PARAM_MARKER);
  if (firstParameter < 0) return { id: value, parameters: new Map() };

  const parameters = new Map<string, string>();
  for (const part of value.slice(firstParameter + CURSOR_PARAM_MARKER.length).split(CURSOR_PARAM_MARKER)) {
    const separator = part.indexOf(':');
    if (separator <= 0) continue;
    try {
      parameters.set(decodeURIComponent(part.slice(0, separator)), decodeURIComponent(part.slice(separator + 1)));
    } catch {
      return { id: value, parameters: new Map() };
    }
  }
  return { id: value.slice(0, firstParameter), parameters };
}

function serializeCursorSelection(selection: CursorSelection): string {
  const parameters = [...selection.parameters.entries()].sort(([a], [b]) => a.localeCompare(b));
  return `${selection.id}${parameters
    .map(([id, value]) => `${CURSOR_PARAM_MARKER}${encodeURIComponent(id)}:${encodeURIComponent(value)}`)
    .join('')}`;
}

function displayCursorOption(parameter: string, value: string): string {
  if (parameter === 'fast') {
    if (value === 'fast' || value === 'true') return 'Yes';
    if (value === 'false') return 'No';
  }
  if (parameter === 'thinking') return value === 'true' ? 'Enabled' : 'Disabled';
  return value.replace(/-/g, ' ');
}

function humanizeCursorModelId(id: string): string {
  const acronyms = new Map([
    ['ai', 'AI'],
    ['glm', 'GLM'],
    ['gpt', 'GPT'],
    ['kimi', 'Kimi'],
  ]);
  return id
    .split(/[-_]/)
    .filter(Boolean)
    .map(part => acronyms.get(part.toLowerCase()) ?? `${part.slice(0, 1).toUpperCase()}${part.slice(1)}`)
    .join(' ');
}

const CursorModelSelector = ({
  model,
  onModelChange,
  catalog,
  label,
}: {
  model: string;
  onModelChange: (next: string) => void;
  catalog: readonly ModelInfo[];
  label: string;
}) => {
  const models = useMemo(() => {
    const grouped = new Map<string, CursorModelCatalog>();
    for (const entry of catalog) {
      const selection = parseCursorSelection(entry.id);
      const current = grouped.get(selection.id) ?? { id: selection.id, parameters: new Map() };
      for (const [parameter, value] of selection.parameters) {
        const values = current.parameters.get(parameter) ?? new Set<string>();
        values.add(value);
        current.parameters.set(parameter, values);
      }
      grouped.set(selection.id, current);
    }
    return [...grouped.values()].sort((a, b) => a.id.localeCompare(b.id));
  }, [catalog]);

  const selection = parseCursorSelection(model);
  const active = models.find(entry => entry.id === selection.id);
  const updateParameter = (parameter: string, value: string) => {
    const next = new Map(selection.parameters);
    if (value) next.set(parameter, value);
    else next.delete(parameter);
    onModelChange(serializeCursorSelection({ id: selection.id, parameters: next }));
  };

  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-col gap-1.5">
        <SettingsSelect
          aria-label={label}
          value={selection.id}
          onChange={event => onModelChange(event.target.value)}
          className="w-full">
          {!selection.id && <option value="">Select a model</option>}
          {selection.id && !models.some(entry => entry.id === selection.id) && (
            <option value={selection.id}>{selection.id}</option>
          )}
          {models.map(entry => (
            <option key={entry.id} value={entry.id}>
              {humanizeCursorModelId(entry.id)} — {entry.id}
            </option>
          ))}
        </SettingsSelect>
      </div>

      {active &&
        [...active.parameters.entries()]
          .sort(([a], [b]) => cursorParameterOrder(a) - cursorParameterOrder(b) || a.localeCompare(b))
          .map(([parameter, values]) => (
            <div key={parameter} className="flex flex-col gap-1.5">
              <label className="text-xs font-medium text-content-secondary">
                {CURSOR_PARAMETER_LABELS[parameter] ?? parameter}
              </label>
              <SettingsSelect
                aria-label={`${label} ${parameter}`}
                value={selection.parameters.get(parameter) ?? ''}
                onChange={event => updateParameter(parameter, event.target.value)}
                className="w-full">
                <option value="">Default</option>
                {[...values]
                  .sort((a, b) => a.localeCompare(b))
                  .map(value => (
                    <option key={value} value={value}>
                      {displayCursorOption(parameter, value)}
                    </option>
                  ))}
              </SettingsSelect>
            </div>
          ))}
    </div>
  );
};

/** Resolved entry-mode state for one picker. Produced by {@link useModelEntryMode}. */
export interface ModelEntryMode {
  /** The selected provider's endpoint host is Azure — this field is a deployment name. */
  isAzureProvider: boolean;
  /** The user's explicit choice, or the Azure-derived default when untouched. */
  manualEntry: boolean;
  /** Effective mode: free text when asked for, or when there is no catalog to pick from. */
  useManualEntry: boolean;
  /** The stored value is verbatim a catalog entry — the fingerprint of a pre-fix selection. */
  showAzureLegacyHint: boolean;
  /** Flip between the catalog dropdown and free text. */
  toggleManualEntry: () => void;
  /**
   * Re-derive the default mode for a newly selected provider. Call this from
   * the provider `<select>` handler with the next provider's endpoint (or
   * `undefined` for a non-cloud source such as local Ollama / Claude Code).
   */
  syncToEndpoint: (endpoint: string | null | undefined) => void;
}

/**
 * Own the Azure detection + entry-mode state for a model picker.
 *
 * `endpoint` is the currently selected cloud provider's endpoint; pass
 * `undefined` when the selected source is not a cloud provider.
 */
export function useModelEntryMode({
  endpoint,
  model,
  catalogIds,
}: {
  endpoint: string | null | undefined;
  model: string;
  catalogIds: readonly string[];
}): ModelEntryMode {
  const isAzureProvider = isAzureFoundryEndpoint(endpoint);
  // Seeded from the initially selected provider; re-seeded by `syncToEndpoint`
  // whenever the user picks a different one, and overridden by the toggle.
  const [manualEntry, setManualEntry] = useState<boolean>(() => isAzureFoundryEndpoint(endpoint));

  const toggleManualEntry = useCallback(() => setManualEntry(v => !v), []);
  const syncToEndpoint = useCallback((next: string | null | undefined) => {
    setManualEntry(isAzureFoundryEndpoint(next));
  }, []);

  return {
    isAzureProvider,
    manualEntry,
    // An empty catalog leaves nothing to pick from — that was already the
    // pre-existing behaviour for a provider whose listing came back empty.
    useManualEntry: manualEntry || catalogIds.length === 0,
    showAzureLegacyHint: isAzureProvider && looksLikeAzureBaseModelId(model, catalogIds),
    toggleManualEntry,
    syncToEndpoint,
  };
}

/**
 * Render the model / deployment-name field: catalog dropdown or free text, the
 * mode toggle, the loading + probe-error branches, and the Azure guidance.
 *
 * The caller keeps ownership of the non-cloud branches (installed local models,
 * the Claude Code alias field) — those have their own option sources and are
 * not part of the Azure story.
 */
export const ModelEntryField = ({
  mode,
  model,
  onModelChange,
  catalog,
  catalogLoading,
  catalogError,
  onRetry,
  label,
  placeholder,
  analyticsId,
  optionLabel,
  providerSlug,
}: {
  mode: ModelEntryMode;
  model: string;
  onModelChange: (next: string) => void;
  catalog: readonly ModelInfo[];
  catalogLoading?: boolean;
  catalogError?: string | null;
  onRetry?: () => void;
  /** Field label used when the provider is not Azure. */
  label: string;
  /** Free-text placeholder used when the provider is not Azure. */
  placeholder: string;
  analyticsId: string;
  /** Option text for a catalog entry. Defaults to the bare model id. */
  optionLabel?: (m: ModelInfo) => string;
  /** Enables Cursor's grouped model-parameter picker for the local bridge. */
  providerSlug?: string | null;
}) => {
  const { t } = useT();
  const { isAzureProvider, manualEntry, useManualEntry, showAzureLegacyHint } = mode;
  const fieldLabel = isAzureProvider ? t('settings.ai.deploymentNameLabel') : label;

  // A still-loading catalog only blocks the dropdown. Gate on the *explicit*
  // mode rather than the effective one: while the probe is in flight the
  // catalog is empty, so `useManualEntry` is transiently true for everyone and
  // gating on it would drop the "loading" affordance entirely. In free-text
  // mode (every Azure connection) the field is usable immediately — waiting on
  // a listing whose values are the wrong ones anyway would be pure delay.
  const showLoadingSelect = Boolean(catalogLoading) && !manualEntry;

  return (
    <div className="flex flex-col gap-1.5">
      <label className="text-xs font-medium text-content-secondary">{fieldLabel}</label>

      {catalogError ? (
        <div className="rounded-lg border border-red-200 dark:border-red-500/30 bg-red-50 dark:bg-red-500/10 px-3 py-2 text-xs text-red-700 dark:text-red-300 font-mono break-all">
          {catalogError}
        </div>
      ) : null}
      {catalogError && onRetry ? (
        <div className="flex items-center gap-2">
          <Button type="button" variant="tertiary" size="xs" onClick={onRetry}>
            {t('common.retry')}
          </Button>
          {/* Azure gets `deploymentNameHelp` under the field instead — this
              copy names a model id, which is the wrong thing to ask for. */}
          {!isAzureProvider && (
            <span className="text-xs text-content-faint">
              {t('settings.ai.enterModelIdManually')}
            </span>
          )}
        </div>
      ) : null}

      {showLoadingSelect ? (
        <SettingsSelect disabled className="w-full opacity-60 cursor-wait">
          <option>{t('settings.ai.loadingModels')}</option>
        </SettingsSelect>
      ) : useManualEntry ? (
        <SettingsTextField
          type="text"
          mono
          aria-label={fieldLabel}
          value={model}
          onChange={e => onModelChange(e.target.value)}
          placeholder={isAzureProvider ? t('settings.ai.deploymentNamePlaceholder') : placeholder}
        />
      ) : providerSlug === 'cursor' ? (
        <CursorModelSelector model={model} onModelChange={onModelChange} catalog={catalog} label={fieldLabel} />
      ) : (
        <SettingsSelect
          aria-label={fieldLabel}
          value={model}
          onChange={e => onModelChange(e.target.value)}
          className="w-full">
          {!model && <option value="">{t('settings.ai.selectModel')}</option>}
          {/* Keep an off-catalog value (e.g. a deployment name) selectable so
              switching to the dropdown can never silently drop it. */}
          {model && !catalog.some(m => m.id === model) && <option value={model}>{model}</option>}
          {catalog.map(m => (
            <option key={m.id} value={m.id}>
              {optionLabel ? optionLabel(m) : m.id}
            </option>
          ))}
        </SettingsSelect>
      )}

      {/* Escape hatch out of the catalog: a deployment name (Azure) or any model
          the provider does not advertise is otherwise unreachable. Pointless
          when there is no catalog — free text is already the only mode. */}
      {catalog.length > 0 && (
        <Button
          type="button"
          variant="tertiary"
          size="xs"
          analyticsId={analyticsId}
          onClick={mode.toggleManualEntry}>
          {manualEntry
            ? t('settings.ai.chooseModelFromList')
            : isAzureProvider
              ? t('settings.ai.enterDeploymentNameManuallyAction')
              : t('settings.ai.enterModelIdManuallyAction')}
        </Button>
      )}

      {isAzureProvider && (
        <p className="text-[11px] text-content-muted">{t('settings.ai.deploymentNameHelp')}</p>
      )}
      {showAzureLegacyHint && (
        <p className="rounded-lg border border-amber-200 dark:border-amber-500/30 bg-amber-50 dark:bg-amber-500/10 px-3 py-2 text-[11px] text-amber-800 dark:text-amber-200">
          {t('settings.ai.deploymentNameLegacyHint')}
        </p>
      )}
    </div>
  );
};

export default ModelEntryField;
