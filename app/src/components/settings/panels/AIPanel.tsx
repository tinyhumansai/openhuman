/*
 * AI settings — three orthogonal sections:
 *   1. Cloud providers (credentials + primary selection)
 *   2. Local provider (Ollama runtime + installed models)
 *   3. Workload routing (8-row matrix; per-workload provider + model)
 *
 * "Primary cloud" is an abstraction: any workload set to "Primary" inherits
 * whichever cloud provider is currently marked primary. Overrides are explicit
 * per row, so the resolved provider+model is always rendered inline.
 */
import { type ReactElement, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { LuCheck, LuCircleAlert, LuCloud, LuServer, LuShield, LuWand, LuZap } from 'react-icons/lu';

import {
  type AISettings as ApiAISettings,
  type ProviderRef as ApiProviderRef,
  cacheProviderModelIds,
  clearCloudProviderKey,
  clearProviderModelIds,
  type CloudProviderView,
  loadAISettings,
  loadLocalProviderSnapshot,
  loadProviderModelIds,
  type LocalProviderSnapshot,
  saveAISettings,
  setCloudProviderKey,
  validateCloudProviderKey,
} from '../../../services/api/aiSettingsApi';
import type { CloudProviderType as ApiCloudProviderType } from '../../../utils/tauriCommands/config';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

type CloudProviderType = 'openhuman' | 'openai' | 'anthropic' | 'openrouter' | 'custom';

type CloudProvider = {
  id: string;
  type: CloudProviderType;
  label: string;
  endpoint: string;
  maskedKey: string;
  defaultModel: string;
};

type OllamaState = 'disabled' | 'missing' | 'stopped' | 'starting' | 'running' | 'error';

type OllamaModel = { id: string; sizeBytes: number; family: string };

type WorkloadId =
  | 'reasoning'
  | 'agentic'
  | 'coding'
  | 'memory'
  | 'embeddings'
  | 'heartbeat'
  | 'learning'
  | 'subconscious';

type WorkloadGroup = 'chat' | 'background';

type ProviderRef =
  | { kind: 'primary' }
  | { kind: 'cloud'; providerId: string; model: string }
  | { kind: 'local'; model: string };

type Workload = { id: WorkloadId; group: WorkloadGroup; label: string; description: string };

type RoutingMap = Record<WorkloadId, ProviderRef>;

// ─────────────────────────────────────────────────────────────────────────────
// Static catalog
// ─────────────────────────────────────────────────────────────────────────────

const WORKLOADS: Workload[] = [
  {
    id: 'reasoning',
    group: 'chat',
    label: 'Reasoning',
    description: 'Main chat agent, meeting summarizer',
  },
  {
    id: 'agentic',
    group: 'chat',
    label: 'Agentic',
    description: 'Sub-agent runners, tool loops, GIF decisions',
  },
  {
    id: 'coding',
    group: 'chat',
    label: 'Coding',
    description: 'Code generation and refactor passes',
  },
  {
    id: 'memory',
    group: 'background',
    label: 'Memory summarization',
    description: 'Tree-extracts and consolidations',
  },
  {
    id: 'embeddings',
    group: 'background',
    label: 'Embeddings',
    description: 'Vector encoding for memory retrieval',
  },
  {
    id: 'heartbeat',
    group: 'background',
    label: 'Heartbeat',
    description: 'Background reasoning between user turns',
  },
  {
    id: 'learning',
    group: 'background',
    label: 'Learning · Reflections',
    description: 'Periodic reflection over recent history',
  },
  {
    id: 'subconscious',
    group: 'background',
    label: 'Subconscious',
    description: 'Eventfulness scoring + drift checks',
  },
];

const PROVIDER_META: Record<
  CloudProviderType,
  { label: string; rail: string; pill: string; icon: ReactElement }
> = {
  openhuman: {
    label: 'OpenHuman',
    rail: 'bg-primary-500',
    pill: 'bg-primary-50 text-primary-700 ring-primary-200',
    icon: <LuShield className="h-3 w-3" />,
  },
  openai: {
    label: 'OpenAI',
    rail: 'bg-sage-500',
    pill: 'bg-sage-50 text-sage-700 ring-sage-200',
    icon: <LuWand className="h-3 w-3" />,
  },
  anthropic: {
    label: 'Anthropic',
    rail: 'bg-amber-500',
    pill: 'bg-amber-50 text-amber-700 ring-amber-200',
    icon: <LuZap className="h-3 w-3" />,
  },
  openrouter: {
    label: 'OpenRouter',
    rail: 'bg-slate-500',
    pill: 'bg-slate-50 text-slate-700 ring-slate-200',
    icon: <LuCloud className="h-3 w-3" />,
  },
  custom: {
    label: 'Custom',
    rail: 'bg-stone-500',
    pill: 'bg-stone-100 text-stone-700 ring-stone-200',
    icon: <LuServer className="h-3 w-3" />,
  },
};

// TIER_PRESETS removed alongside the Local provider section.

// ─────────────────────────────────────────────────────────────────────────────
// API-adapter hooks
//
// The panel works in terms of `CloudProvider` (with a derived `label` +
// `maskedKey`) and `ProviderRef.cloud.providerId`. The wire format uses
// provider TYPE, not id. These hooks bridge the two.
// ─────────────────────────────────────────────────────────────────────────────

type AISettings = { cloudProviders: CloudProvider[]; primaryCloudId: string; routing: RoutingMap };

const EMPTY_SETTINGS: AISettings = {
  cloudProviders: [],
  primaryCloudId: '',
  routing: {
    reasoning: { kind: 'primary' },
    agentic: { kind: 'primary' },
    coding: { kind: 'primary' },
    memory: { kind: 'primary' },
    embeddings: { kind: 'primary' },
    heartbeat: { kind: 'primary' },
    learning: { kind: 'primary' },
    subconscious: { kind: 'primary' },
  },
};

function maskKeyLabel(hasKey: boolean): string {
  return hasKey ? '•••• configured' : 'Not configured';
}

function toPanelProvider(p: CloudProviderView): CloudProvider {
  return {
    id: p.id,
    type: p.type as CloudProviderType,
    label: PROVIDER_META[p.type as CloudProviderType].label,
    endpoint: p.endpoint,
    maskedKey: maskKeyLabel(p.has_api_key),
    defaultModel: p.default_model,
  };
}

function toPanelRoutingFromApi(api: ApiAISettings): { panel: AISettings } {
  const cloudProviders = api.cloudProviders.map(toPanelProvider);
  const idByType = new Map<string, string>();
  for (const p of cloudProviders) {
    idByType.set(p.type, p.id);
  }
  const liftRef = (r: ApiProviderRef): ProviderRef => {
    if (r.kind === 'primary') return { kind: 'primary' };
    if (r.kind === 'local') return { kind: 'local', model: r.model };
    // cloud
    const id = idByType.get(r.providerType) ?? '';
    if (!id) {
      // Provider type referenced but no entry — degrade to primary.
      return { kind: 'primary' };
    }
    return { kind: 'cloud', providerId: id, model: r.model };
  };
  const routing: RoutingMap = {
    reasoning: liftRef(api.routing.reasoning),
    agentic: liftRef(api.routing.agentic),
    coding: liftRef(api.routing.coding),
    memory: liftRef(api.routing.memory),
    embeddings: liftRef(api.routing.embeddings),
    heartbeat: liftRef(api.routing.heartbeat),
    learning: liftRef(api.routing.learning),
    subconscious: liftRef(api.routing.subconscious),
  };
  return {
    panel: {
      cloudProviders,
      primaryCloudId: api.primaryCloudId ?? cloudProviders[0]?.id ?? '',
      routing,
    },
  };
}

function toApiRefFromPanel(r: ProviderRef, providers: CloudProvider[]): ApiProviderRef {
  if (r.kind === 'primary') return { kind: 'primary' };
  if (r.kind === 'local') return { kind: 'local', model: r.model };
  const entry = providers.find(p => p.id === r.providerId);
  if (!entry) return { kind: 'primary' };
  return { kind: 'cloud', providerType: entry.type as ApiCloudProviderType, model: r.model };
}

function toApiSettings(panel: AISettings): ApiAISettings {
  return {
    cloudProviders: panel.cloudProviders.map(p => ({
      id: p.id,
      type: p.type as ApiCloudProviderType,
      endpoint: p.endpoint,
      default_model: p.defaultModel,
      has_api_key: p.maskedKey.startsWith('••••'),
    })),
    primaryCloudId: panel.primaryCloudId || null,
    routing: {
      reasoning: toApiRefFromPanel(panel.routing.reasoning, panel.cloudProviders),
      agentic: toApiRefFromPanel(panel.routing.agentic, panel.cloudProviders),
      coding: toApiRefFromPanel(panel.routing.coding, panel.cloudProviders),
      memory: toApiRefFromPanel(panel.routing.memory, panel.cloudProviders),
      embeddings: toApiRefFromPanel(panel.routing.embeddings, panel.cloudProviders),
      heartbeat: toApiRefFromPanel(panel.routing.heartbeat, panel.cloudProviders),
      learning: toApiRefFromPanel(panel.routing.learning, panel.cloudProviders),
      subconscious: toApiRefFromPanel(panel.routing.subconscious, panel.cloudProviders),
    },
  };
}

function useAISettings() {
  const [saved, setSaved] = useState<AISettings>(EMPTY_SETTINGS);
  const [draft, setDraft] = useState<AISettings>(EMPTY_SETTINGS);
  const [loading, setLoading] = useState<boolean>(true);
  const [error, setError] = useState<string>('');

  const reload = useCallback(async () => {
    setLoading(true);
    setError('');
    try {
      const api = await loadAISettings();
      const { panel } = toPanelRoutingFromApi(api);
      setSaved(panel);
      setDraft(panel);
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to load AI settings';
      setError(message);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  const isDirty = JSON.stringify(saved) !== JSON.stringify(draft);

  const save = useCallback(async () => {
    try {
      await saveAISettings(toApiSettings(saved), toApiSettings(draft));
      setSaved(draft);
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to save AI settings';
      setError(message);
    }
  }, [saved, draft]);

  const discard = useCallback(() => setDraft(saved), [saved]);

  return { saved, draft, setDraft, isDirty, save, discard, loading, error, reload };
}

function useOllamaStatus() {
  const [snapshot, setSnapshot] = useState<LocalProviderSnapshot | null>(null);
  const lastPollRef = useRef<number>(0);

  const refresh = useCallback(async (): Promise<LocalProviderSnapshot | null> => {
    try {
      const s = await loadLocalProviderSnapshot();
      setSnapshot(s);
      lastPollRef.current = Date.now();
      return s;
    } catch {
      // Swallow — keep last good snapshot, return null so callers can
      // detect failure without a try/catch.
      return null;
    }
  }, []);

  useEffect(() => {
    void refresh();
    const id = window.setInterval(() => void refresh(), 5000);
    return () => window.clearInterval(id);
  }, [refresh]);

  // Translate to the OllamaState the panel UI expects.
  //
  // `disabled` is the config-side master switch (user turned local AI off
  // via the toggle). `missing` is "user wants local AI but the daemon
  // isn't installed". Keep them distinct so the toggle's `checked` state
  // and the Install/Retry button can render the right thing.
  const state: OllamaState = useMemo(() => {
    if (!snapshot) return 'stopped';
    const stateStr = snapshot.status?.state ?? '';
    if (stateStr === 'disabled') return 'disabled';
    if (snapshot.diagnostics?.ollama_running) return 'running';
    if (stateStr === 'missing') return 'missing';
    if (stateStr === 'starting' || stateStr === 'downloading') return 'starting';
    if (stateStr === 'error') return 'error';
    return 'stopped';
  }, [snapshot]);

  const version = snapshot?.diagnostics?.ollama_binary_path
    ? // Diagnostics doesn't surface a version string today; show the binary path tail.
      (snapshot.diagnostics.ollama_binary_path.split(/[\\/]/).pop() ?? '')
    : '';

  return { state, version, snapshot, refresh };
}

function useInstalledModels(snapshot: LocalProviderSnapshot | null): OllamaModel[] {
  return useMemo(() => {
    const list = snapshot?.installedModels ?? [];
    return list.map(m => ({
      id: m.name,
      sizeBytes: m.size ?? 0,
      family: m.name.split(/[:/]/, 1)[0] ?? 'model',
    }));
  }, [snapshot]);
}

// ─────────────────────────────────────────────────────────────────────────────
// Primitives
// ─────────────────────────────────────────────────────────────────────────────

// SectionLabel removed alongside its only call site (the old
// "Cloud providers" / "Local provider" headings).

// formatBytes / StatusDot / ProviderChip helpers removed alongside the
// Local provider section + CloudProviderCard — no callers left.

// ─────────────────────────────────────────────────────────────────────────────
// Cloud provider card
// ─────────────────────────────────────────────────────────────────────────────

// Provider "type" id used by the chip UI. Extends CloudProviderType with two
// local-runtime brands (LM Studio + Ollama) so they get the same chip
// affordance. Backend storage still uses CloudProviderType; the two extras
// are persisted as `type: 'custom'` with a distinguishing label.
type ProviderChipType = CloudProviderType | 'lmstudio' | 'ollama';

// Faint brand-tinted background per provider. Compact (no icon) chip that
// holds the provider name + a small toggle switch. The brand tint is shown
// at all times (enabled and disabled) — the toggle is the only enabled-state
// signal — so the row reads as a row of branded options at a glance.
//
// Tints are rough approximations of each vendor's brand:
//   - OpenAI     → emerald (their classic green logo accent)
//   - Anthropic  → orange  (Claude / Anthropic warm orange branding)
//   - OpenRouter → slate   (dark / monochrome logo)
//   - LM Studio  → cyan    (their teal/cyan icon)
//   - Ollama     → violet  (their llama-themed purple tone)
//   - Custom     → stone   (neutral)
const PROVIDER_CHIP_TONE: Record<ProviderChipType, string> = {
  openhuman: 'bg-primary-50 ring-primary-200 text-primary-900',
  openai: 'bg-emerald-50 ring-emerald-200 text-emerald-900',
  anthropic: 'bg-orange-50 ring-orange-200 text-orange-900',
  openrouter: 'bg-slate-100 ring-slate-300 text-slate-900',
  lmstudio: 'bg-cyan-50 ring-cyan-200 text-cyan-900',
  ollama: 'bg-violet-50 ring-violet-200 text-violet-900',
  custom: 'bg-stone-100 ring-stone-300 text-stone-900',
};

const PROVIDER_CHIP_LABEL: Record<ProviderChipType, string> = {
  openhuman: 'OpenHuman',
  openai: 'OpenAI',
  anthropic: 'Anthropic',
  openrouter: 'OpenRouter',
  lmstudio: 'LM Studio',
  ollama: 'Ollama',
  custom: 'Custom',
};

const ProviderToggleChip = ({
  type,
  label,
  enabled,
  busy,
  onToggle,
}: {
  type: ProviderChipType;
  label: string;
  enabled: boolean;
  busy?: boolean;
  onToggle: () => void;
}) => {
  const tone = PROVIDER_CHIP_TONE[type];
  return (
    <div
      className={`inline-flex items-center gap-2 rounded-full px-2.5 py-1 text-xs font-medium ring-1 transition-colors ${tone}`}>
      <span>{label}</span>
      <button
        type="button"
        role="switch"
        aria-checked={enabled}
        aria-label={`${enabled ? 'Disconnect' : 'Connect'} ${label}`}
        disabled={busy}
        onClick={onToggle}
        className={`relative inline-flex h-4 w-7 shrink-0 items-center rounded-full transition-colors disabled:cursor-wait disabled:opacity-60 ${
          enabled ? 'bg-primary-500' : 'bg-stone-300'
        }`}>
        <span
          aria-hidden
          className={`inline-block h-3 w-3 transform rounded-full bg-white shadow transition-transform ${
            enabled ? 'translate-x-3.5' : 'translate-x-0.5'
          }`}
        />
      </button>
    </div>
  );
};

// Minimal API-key dialog — shown when the user flips a provider toggle ON.
// No endpoint / model fields; default endpoint is derived from the type and
// the model is left empty (the routing dialog picks the model per workload).
const ProviderKeyDialog = ({
  type,
  label,
  onCancel,
  onSubmit,
}: {
  type: CloudProviderType;
  label: string;
  onCancel: () => void;
  onSubmit: (apiKey: string, modelIds?: string[]) => Promise<void> | void;
}) => {
  const [apiKey, setApiKey] = useState('');
  const [phase, setPhase] = useState<'idle' | 'testing' | 'saving'>('idle');
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);
  const busy = phase !== 'idle';

  const placeholder =
    type === 'openai'
      ? 'sk-...'
      : type === 'anthropic'
        ? 'sk-ant-...'
        : type === 'openrouter'
          ? 'sk-or-...'
          : 'your-api-key';

  const handleSave = async () => {
    const trimmed = apiKey.trim();
    if (!trimmed) {
      setError('Please paste your API key to continue.');
      return;
    }
    setError(null);
    setSuccess(null);

    // Sanity-check the key against the provider's models endpoint before
    // we persist anything. For provider types we don't know how to verify
    // (custom / local runtimes) `validateCloudProviderKey` resolves
    // `{ ok: true }` without making a request, so this is a no-op there.
    setPhase('testing');
    const result = await validateCloudProviderKey(type, trimmed);
    if (!result.ok) {
      setError(result.error ?? "Couldn't verify that key. Please try again.");
      setPhase('idle');
      return;
    }
    if (typeof result.modelCount === 'number') {
      setSuccess(`Key looks good — ${result.modelCount} models available.`);
    }

    setPhase('saving');
    try {
      await onSubmit(trimmed, result.modelIds);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setPhase('idle');
    }
  };

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label={`Connect ${label}`}
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/30 p-4">
      <div className="w-full max-w-md rounded-2xl border border-stone-200 bg-white p-6 shadow-soft">
        <div className="mb-4">
          <h3 className="text-base font-semibold text-stone-900">Connect {label}</h3>
          <p className="mt-0.5 text-xs text-stone-500">
            Paste your API key. It's stored encrypted on this device only.
          </p>
        </div>

        <div className="flex flex-col gap-1.5">
          <label htmlFor="provider-key-input" className="text-xs font-medium text-stone-700">
            API key
          </label>
          <input
            id="provider-key-input"
            type="text"
            autoComplete="off"
            autoCorrect="off"
            autoCapitalize="off"
            spellCheck={false}
            data-form-type="other"
            data-lpignore="true"
            data-1p-ignore="true"
            value={apiKey}
            placeholder={placeholder}
            disabled={busy}
            onChange={e => {
              setApiKey(e.target.value);
              setError(null);
              setSuccess(null);
            }}
            className="rounded-lg border border-stone-300 bg-white px-3 py-2 text-sm text-stone-900 placeholder-stone-400 focus:border-primary-500 focus:outline-none focus:ring-1 focus:ring-primary-500 disabled:opacity-60"
          />
          {error ? <p className="text-xs font-medium text-red-600">{error}</p> : null}
          {success && !error ? (
            <p className="text-xs font-medium text-emerald-600">{success}</p>
          ) : null}
        </div>

        <div className="mt-6 flex justify-end gap-2">
          <button
            type="button"
            onClick={onCancel}
            disabled={busy}
            className="rounded-lg border border-stone-200 bg-white px-4 py-2 text-sm font-medium text-stone-700 hover:bg-stone-50 disabled:opacity-50">
            Cancel
          </button>
          <button
            type="button"
            onClick={() => void handleSave()}
            disabled={busy}
            className="rounded-lg bg-primary-500 px-4 py-2 text-sm font-medium text-white hover:bg-primary-600 disabled:cursor-not-allowed disabled:opacity-50">
            {phase === 'testing' ? 'Testing…' : phase === 'saving' ? 'Saving…' : 'Save'}
          </button>
        </div>
      </div>
    </div>
  );
};

// CloudProviderCard was removed alongside the list-based auth UI. The new
// chip layout (ProviderToggleChip) covers the same affordances with less
// chrome. CloudProviderEditor still exists for the advanced add/edit flow,
// although nothing currently mounts it.

// ─────────────────────────────────────────────────────────────────────────────
// Workload row (stacked, narrow-friendly)
// ─────────────────────────────────────────────────────────────────────────────

type WorkloadRowProps = {
  workload: Workload;
  ref_: ProviderRef;
  primary: CloudProvider | undefined;
  cloudProviders: CloudProvider[];
  localModels: OllamaModel[];
  ollamaState: OllamaState;
  onChange: (next: ProviderRef) => void;
};

const WorkloadRow = ({
  workload,
  ref_,
  primary,
  cloudProviders,
  localModels,
  ollamaState,
  onChange,
  onCustomClick,
}: WorkloadRowProps & { onCustomClick: () => void }) => {
  const selectedCloud =
    ref_.kind === 'cloud' ? cloudProviders.find(c => c.id === ref_.providerId) : undefined;

  const isDefault = ref_.kind === 'primary';

  let resolved: string;
  if (ref_.kind === 'primary') {
    if (!primary) resolved = 'no primary set';
    else if (primary.type === 'openhuman') resolved = 'OpenHuman';
    else resolved = `${primary.label} · ${primary.defaultModel}`;
  } else if (ref_.kind === 'cloud') {
    if (!selectedCloud) resolved = ref_.model;
    else if (selectedCloud.type === 'openhuman') resolved = 'OpenHuman';
    else resolved = `${selectedCloud.label} · ${ref_.model}`;
  } else {
    resolved = `Ollama · ${ref_.model}`;
  }

  // Quiet `ollamaState` / `localModels` unused-prop warnings — they're still
  // consumed by the parent's onChange wiring through `onCustomClick`.
  void ollamaState;
  void localModels;

  const segmentBase =
    'flex-1 px-3 py-1.5 text-xs font-medium rounded-md transition-colors cursor-pointer';
  const activeSegment = 'bg-white text-stone-900 shadow-subtle ring-1 ring-stone-200';
  const inactiveSegment = 'text-stone-500 hover:text-stone-800';

  return (
    <div className="flex items-center justify-between gap-3 py-3">
      <div className="min-w-0 flex-1">
        <div className="text-sm font-medium text-stone-900">{workload.label}</div>
        <div className="truncate text-xs text-stone-500">{workload.description}</div>
        <div className="mt-0.5 font-mono text-[11px] text-stone-400 truncate">↳ {resolved}</div>
      </div>
      <div className="inline-flex shrink-0 items-center rounded-lg bg-stone-100 p-0.5">
        <button
          type="button"
          onClick={() => onChange({ kind: 'primary' })}
          className={`${segmentBase} ${isDefault ? activeSegment : inactiveSegment}`}>
          Default
        </button>
        <button
          type="button"
          onClick={onCustomClick}
          className={`${segmentBase} ${!isDefault ? activeSegment : inactiveSegment}`}>
          Custom
        </button>
      </div>
    </div>
  );
};

// ─────────────────────────────────────────────────────────────────────────────
// Custom-routing dialog — opened when the user clicks "Custom" on a workload.
// Lets them pick a provider (cloud or local) and the specific model id.
// ─────────────────────────────────────────────────────────────────────────────

interface CustomRoutingDialogProps {
  workload: Workload;
  initial: ProviderRef;
  cloudProviders: CloudProvider[];
  localModels: OllamaModel[];
  ollamaRunning: boolean;
  /** Per-provider-type model id catalog cached from the validation step.
   *  Empty array for a given type means "no cache, fall back to free-text
   *  model input". */
  cloudModelIds: Partial<Record<CloudProviderType, string[]>>;
  onClose: () => void;
  onSubmit: (next: ProviderRef) => void;
}

type CustomDialogSource = { kind: 'cloud'; providerId: string } | { kind: 'local' };

const CustomRoutingDialog = ({
  workload,
  initial,
  cloudProviders,
  localModels,
  ollamaRunning,
  cloudModelIds,
  onClose,
  onSubmit,
}: CustomRoutingDialogProps) => {
  // Non-openhuman cloud providers + local-ollama (if available) are the
  // "Custom" options. OpenHuman is excluded — it's the Default path.
  const customCloud = cloudProviders.filter(p => p.type !== 'openhuman');
  const localAvailable = ollamaRunning && localModels.length > 0;

  const initialSource: CustomDialogSource | null =
    initial.kind === 'cloud'
      ? { kind: 'cloud', providerId: initial.providerId }
      : initial.kind === 'local'
        ? { kind: 'local' }
        : customCloud[0]
          ? { kind: 'cloud', providerId: customCloud[0].id }
          : localAvailable
            ? { kind: 'local' }
            : null;

  const [source, setSource] = useState<CustomDialogSource | null>(initialSource);
  const [model, setModel] = useState<string>(() => {
    if (initial.kind === 'cloud' || initial.kind === 'local') return initial.model;
    if (initialSource?.kind === 'cloud') {
      const p = customCloud.find(c => c.id === initialSource.providerId);
      return p?.defaultModel ?? '';
    }
    return localModels[0]?.id ?? '';
  });

  const selectedCloud =
    source?.kind === 'cloud' ? customCloud.find(c => c.id === source.providerId) : undefined;

  const canSave = source !== null && model.trim().length > 0;

  const handleSave = () => {
    if (!source || !canSave) return;
    if (source.kind === 'cloud') {
      onSubmit({ kind: 'cloud', providerId: source.providerId, model: model.trim() });
    } else {
      onSubmit({ kind: 'local', model: model.trim() });
    }
  };

  const noProviders = customCloud.length === 0 && !localAvailable;

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label={`Custom routing for ${workload.label}`}
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/30 p-4">
      <div className="w-full max-w-md rounded-2xl border border-stone-200 bg-white p-6 shadow-soft">
        <div className="flex items-start justify-between gap-3 mb-4">
          <div>
            <h3 className="text-base font-semibold text-stone-900">Custom routing</h3>
            <p className="mt-0.5 text-xs text-stone-500">{workload.label}</p>
          </div>
          <button
            type="button"
            onClick={onClose}
            className="rounded-md p-1 text-stone-400 hover:bg-stone-100 hover:text-stone-700">
            <span className="sr-only">Close</span>
            <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M6 18L18 6M6 6l12 12"
              />
            </svg>
          </button>
        </div>

        {noProviders ? (
          <div className="rounded-lg border border-amber-200 bg-amber-50 p-3 text-xs text-amber-800">
            No custom providers are set up yet. Add a cloud provider key above, or enable the local
            Ollama runtime, then come back to pick one.
          </div>
        ) : (
          <div className="flex flex-col gap-4">
            <div className="flex flex-col gap-1.5">
              <label className="text-xs font-medium text-stone-700">Provider</label>
              <select
                value={
                  source ? `${source.kind}:${source.kind === 'cloud' ? source.providerId : ''}` : ''
                }
                onChange={e => {
                  const [kind, providerId] = e.target.value.split(':');
                  if (kind === 'local') {
                    setSource({ kind: 'local' });
                    setModel(localModels[0]?.id ?? '');
                  } else if (kind === 'cloud') {
                    const p = customCloud.find(c => c.id === providerId);
                    setSource({ kind: 'cloud', providerId });
                    setModel(p?.defaultModel ?? '');
                  }
                }}
                className="rounded-lg border border-stone-300 bg-white px-3 py-2 text-sm text-stone-900 focus:border-primary-500 focus:outline-none focus:ring-1 focus:ring-primary-500">
                {customCloud.map(p => (
                  <option key={p.id} value={`cloud:${p.id}`}>
                    {p.label}
                  </option>
                ))}
                {localAvailable && <option value="local:">Local (Ollama)</option>}
              </select>
            </div>

            <div className="flex flex-col gap-1.5">
              <label className="text-xs font-medium text-stone-700">Model</label>
              {source?.kind === 'local' ? (
                <select
                  value={model}
                  onChange={e => setModel(e.target.value)}
                  className="rounded-lg border border-stone-300 bg-white px-3 py-2 text-sm text-stone-900 focus:border-primary-500 focus:outline-none focus:ring-1 focus:ring-primary-500">
                  {localModels.map(m => (
                    <option key={m.id} value={m.id}>
                      {m.id}
                    </option>
                  ))}
                </select>
              ) : (
                (() => {
                  const cachedIds =
                    selectedCloud && cloudModelIds[selectedCloud.type]
                      ? (cloudModelIds[selectedCloud.type] ?? [])
                      : [];
                  // When we have a cached model list for this provider
                  // (populated at validation time), show a dropdown. Fall
                  // back to free-text otherwise — e.g. for `custom` /
                  // LM Studio / Ollama where we don't pre-query models.
                  if (cachedIds.length > 0) {
                    // Make sure the currently-selected model id is in the
                    // option list even if it's missing from the cached
                    // catalog (typo, deprecated id, etc.) so the dropdown
                    // never silently swallows the user's choice.
                    const visibleIds = cachedIds.includes(model)
                      ? cachedIds
                      : model
                        ? [model, ...cachedIds]
                        : cachedIds;
                    return (
                      <select
                        value={model}
                        onChange={e => setModel(e.target.value)}
                        className="rounded-lg border border-stone-300 bg-white px-3 py-2 text-sm font-mono text-stone-900 focus:border-primary-500 focus:outline-none focus:ring-1 focus:ring-primary-500">
                        {visibleIds.map(id => (
                          <option key={id} value={id}>
                            {id}
                          </option>
                        ))}
                      </select>
                    );
                  }
                  return (
                    <input
                      type="text"
                      value={model}
                      onChange={e => setModel(e.target.value)}
                      placeholder={selectedCloud?.defaultModel ?? 'model-id'}
                      className="rounded-lg border border-stone-300 bg-white px-3 py-2 text-sm font-mono text-stone-900 placeholder-stone-400 focus:border-primary-500 focus:outline-none focus:ring-1 focus:ring-primary-500"
                    />
                  );
                })()
              )}
            </div>
          </div>
        )}

        <div className="mt-6 flex justify-end gap-2">
          <button
            type="button"
            onClick={onClose}
            className="rounded-lg border border-stone-200 bg-white px-4 py-2 text-sm font-medium text-stone-700 hover:bg-stone-50">
            Cancel
          </button>
          <button
            type="button"
            onClick={handleSave}
            disabled={!canSave}
            className="rounded-lg bg-primary-500 px-4 py-2 text-sm font-medium text-white hover:bg-primary-600 disabled:cursor-not-allowed disabled:opacity-50">
            Save
          </button>
        </div>
      </div>
    </div>
  );
};

// ─────────────────────────────────────────────────────────────────────────────
// Save bar (sticky)
// ─────────────────────────────────────────────────────────────────────────────

const SaveBar = ({
  diffSummary,
  changeCount,
  onSave,
  onDiscard,
}: {
  diffSummary: string[];
  changeCount: number;
  onSave: () => void;
  onDiscard: () => void;
}) => (
  <div className="pointer-events-none sticky bottom-3 z-20 flex justify-center px-4">
    <div className="pointer-events-auto flex w-full items-center gap-2 rounded-lg border border-stone-200 bg-white/95 px-3 py-2 shadow-float backdrop-blur-md animate-fade-up">
      <div className="flex h-6 w-6 shrink-0 items-center justify-center rounded bg-amber-50 text-amber-600">
        <LuCircleAlert className="h-3.5 w-3.5" />
      </div>
      <div className="min-w-0 flex-1">
        <div className="text-xs font-medium text-stone-900">
          {changeCount} unsaved change{changeCount === 1 ? '' : 's'}
        </div>
        <div className="truncate font-mono text-[10px] text-stone-500">
          {diffSummary.slice(0, 2).join(' · ')}
          {diffSummary.length > 2 ? ` · +${diffSummary.length - 2}` : ''}
        </div>
      </div>
      <button
        onClick={onDiscard}
        className="rounded-md border border-stone-200 px-2 py-1 text-xs font-medium text-stone-700 hover:bg-stone-50">
        Discard
      </button>
      <button
        onClick={onSave}
        className="inline-flex items-center gap-1 rounded-md bg-primary-500 px-2.5 py-1 text-xs font-medium text-white hover:bg-primary-600">
        <LuCheck className="h-3 w-3" />
        Save
      </button>
    </div>
  </div>
);

// ─────────────────────────────────────────────────────────────────────────────
// Main panel
// ─────────────────────────────────────────────────────────────────────────────

interface AIPanelProps {
  /** When true, the panel is rendered embedded inside another flow (e.g. the
   *  onboarding custom wizard) and skips its own SettingsHeader chrome so the
   *  host frame's title/back controls aren't duplicated. */
  embedded?: boolean;
}

const AIPanel = ({ embedded = false }: AIPanelProps = {}) => {
  const { navigateBack, breadcrumbs } = useSettingsNavigation();
  const { saved, draft, setDraft, isDirty, save, discard, loading, error, reload } =
    useAISettings();
  const ollama = useOllamaStatus();
  const installed = useInstalledModels(ollama.snapshot);
  const [editing, setEditing] = useState<CloudProvider | 'new' | null>(null);
  const [busyAction, setBusyAction] = useState<string | null>(null);
  // Which workload's "Custom" dialog is currently open (null = closed).
  const [customDialogFor, setCustomDialogFor] = useState<WorkloadId | null>(null);
  // Which provider type's API-key dialog is currently open (null = closed).
  const [keyDialogFor, setKeyDialogFor] = useState<CloudProviderType | null>(null);
  // When the user toggles LM Studio / Ollama (both stored as `custom`), we
  // need to remember which label to attach to the upserted provider so the
  // chip can find it again. Cleared when the dialog closes.
  const [pendingLocalLabel, setPendingLocalLabel] = useState<string | null>(null);

  const primary = useMemo(
    () => draft.cloudProviders.find(p => p.id === draft.primaryCloudId),
    [draft]
  );

  // Per-type cache of model IDs we captured at validation time. Used to
  // populate the model dropdown in CustomRoutingDialog. Recomputed when
  // the set of active cloud providers changes (toggle on/off).
  const cloudModelIdsMap = useMemo(() => {
    const out: Partial<Record<CloudProviderType, string[]>> = {};
    for (const p of draft.cloudProviders) {
      if (p.type === 'openhuman') continue;
      out[p.type] = loadProviderModelIds(p.type);
    }
    return out;
  }, [draft.cloudProviders]);

  const updateRouting = (id: WorkloadId, next: ProviderRef) =>
    setDraft({ ...draft, routing: { ...draft.routing, [id]: next } });

  // applyPreset removed alongside the Cloud / Local / Mixed preset pills —
  // the new Default/Custom binary toggle handles routing per workload.

  const diffSummary = useMemo(() => {
    const out: string[] = [];
    for (const w of WORKLOADS) {
      const a = saved.routing[w.id];
      const b = draft.routing[w.id];
      if (JSON.stringify(a) !== JSON.stringify(b)) {
        const describe = (r: ProviderRef) =>
          r.kind === 'primary'
            ? 'primary'
            : r.kind === 'cloud'
              ? `cloud:${r.model}`
              : `local:${r.model}`;
        out.push(`${w.label} → ${describe(b)}`);
      }
    }
    if (saved.primaryCloudId !== draft.primaryCloudId) {
      const p = draft.cloudProviders.find(cp => cp.id === draft.primaryCloudId);
      out.push(`primary → ${p ? p.label : '—'}`);
    }
    return out;
  }, [saved, draft]);

  const chatRows = WORKLOADS.filter(w => w.group === 'chat');
  const bgRows = WORKLOADS.filter(w => w.group === 'background');

  return (
    <div className="relative">
      {!embedded && (
        <SettingsHeader
          title="LLM"
          showBackButton
          onBack={navigateBack}
          breadcrumbs={breadcrumbs}
        />
      )}

      <div className={embedded ? 'space-y-6' : 'space-y-6 p-4'}>
        {/* ═══════════════════════════════════════════════════════════════
            AUTH — provider authentication (cloud providers + local Ollama
            setup). Everything the user needs to wire a model up.
            ═══════════════════════════════════════════════════════════════ */}
        <div className="space-y-4">
          <div className="border-b border-stone-200 pb-2">
            <h2 className="text-base font-semibold text-stone-900">LLM Providers</h2>
            <p className="text-xs text-stone-500 mt-0.5">
              Connect the language-model backends you want OpenHuman to use. Toggle a provider on to
              add its key; toggle off to disconnect.
            </p>
          </div>

          {/* ─── Provider chip-toggle list ────────────────────────────────── */}
          <section className="space-y-3">
            {loading && <div className="text-xs text-stone-500">Loading…</div>}
            {error && (
              <div className="rounded-md border border-coral-200 bg-coral-50 px-3 py-2 text-xs text-coral-700">
                {error}
              </div>
            )}

            <div className="flex flex-wrap gap-2">
              {(['openai', 'anthropic', 'openrouter', 'custom'] as CloudProviderType[]).map(
                type => {
                  const meta = PROVIDER_META[type];
                  const existing = draft.cloudProviders.find(cp => cp.type === type);
                  const enabled = !!existing;
                  return (
                    <ProviderToggleChip
                      key={type}
                      type={type}
                      label={meta.label}
                      enabled={enabled}
                      busy={busyAction === `toggle-${type}`}
                      onToggle={() => {
                        if (enabled && existing) {
                          // Toggle OFF: remove the provider + scrub any
                          // routing entries that pin to it + drop the
                          // cached model-id list for this provider type.
                          const remaining = draft.cloudProviders.filter(
                            cp => cp.id !== existing.id
                          );
                          const nextPrimaryId =
                            draft.primaryCloudId === existing.id
                              ? (remaining[0]?.id ?? null)
                              : draft.primaryCloudId;
                          const nextRouting = Object.fromEntries(
                            Object.entries(draft.routing).map(([wid, ref]) => [
                              wid,
                              ref.kind === 'cloud' && ref.providerId === existing.id
                                ? { kind: 'primary' as const }
                                : ref,
                            ])
                          ) as typeof draft.routing;
                          setDraft({
                            ...draft,
                            cloudProviders: remaining,
                            primaryCloudId: nextPrimaryId,
                            routing: nextRouting,
                          });
                          clearProviderModelIds(type);
                        } else {
                          // Toggle ON: open the API-key popup. The chip
                          // only flips after the dialog saves.
                          setKeyDialogFor(type);
                        }
                      }}
                    />
                  );
                }
              )}

              {/* LM Studio + Ollama — local runtimes. Stored as `type: 'custom'`
                with a distinguishing label so the existing CloudProvider
                machinery doesn't need a new variant. Toggle ON prompts for
                the local endpoint URL via the same API-key dialog (the
                "key" field doubles as the endpoint here). */}
              {(['lmstudio', 'ollama'] as const).map(localKind => {
                const label = PROVIDER_CHIP_LABEL[localKind];
                const existing = draft.cloudProviders.find(
                  cp => cp.type === 'custom' && cp.label === label
                );
                const enabled = !!existing;
                return (
                  <ProviderToggleChip
                    key={localKind}
                    type={localKind}
                    label={label}
                    enabled={enabled}
                    busy={busyAction === `toggle-${localKind}`}
                    onToggle={() => {
                      if (enabled && existing) {
                        const remaining = draft.cloudProviders.filter(cp => cp.id !== existing.id);
                        const nextPrimaryId =
                          draft.primaryCloudId === existing.id
                            ? (remaining[0]?.id ?? null)
                            : draft.primaryCloudId;
                        const nextRouting = Object.fromEntries(
                          Object.entries(draft.routing).map(([wid, ref]) => [
                            wid,
                            ref.kind === 'cloud' && ref.providerId === existing.id
                              ? { kind: 'primary' as const }
                              : ref,
                          ])
                        ) as typeof draft.routing;
                        setDraft({
                          ...draft,
                          cloudProviders: remaining,
                          primaryCloudId: nextPrimaryId,
                          routing: nextRouting,
                        });
                      } else {
                        setKeyDialogFor('custom');
                        setPendingLocalLabel(label);
                      }
                    }}
                  />
                );
              })}
            </div>
          </section>
        </div>
        {/* end of Auth section */}

        {/* ═══════════════════════════════════════════════════════════════
            ROUTING — which workload uses which model. Each row is a
            binary toggle: Default (let OpenHuman pick) or Custom (opens
            a popup to choose provider + model).
            ═══════════════════════════════════════════════════════════════ */}
        <div className="space-y-4">
          <div className="border-b border-stone-200 pb-2">
            <h2 className="text-base font-semibold text-stone-900">Routing</h2>
            <p className="text-xs text-stone-500 mt-0.5">
              Pick how each workload is served. Default uses OpenHuman; Custom lets you point a
              workload at a specific provider and model.
            </p>
          </div>

          <section className="space-y-3">
            <div className="overflow-hidden rounded-lg border border-stone-200 bg-stone-50 px-3">
              <div className="pt-3">
                <div className="text-[10px] font-semibold uppercase tracking-wide text-stone-400">
                  Chat
                </div>
                <div className="divide-y divide-stone-200">
                  {chatRows.map(w => (
                    <WorkloadRow
                      key={w.id}
                      workload={w}
                      ref_={draft.routing[w.id]}
                      primary={primary}
                      cloudProviders={draft.cloudProviders}
                      localModels={installed}
                      ollamaState={ollama.state}
                      onChange={next => updateRouting(w.id, next)}
                      onCustomClick={() => setCustomDialogFor(w.id)}
                    />
                  ))}
                </div>
              </div>
              <div className="pb-3 pt-3">
                <div className="text-[10px] font-semibold uppercase tracking-wide text-stone-400">
                  Background
                </div>
                <div className="divide-y divide-stone-200">
                  {bgRows.map(w => (
                    <WorkloadRow
                      key={w.id}
                      workload={w}
                      ref_={draft.routing[w.id]}
                      primary={primary}
                      cloudProviders={draft.cloudProviders}
                      localModels={installed}
                      ollamaState={ollama.state}
                      onChange={next => updateRouting(w.id, next)}
                      onCustomClick={() => setCustomDialogFor(w.id)}
                    />
                  ))}
                </div>
              </div>
            </div>

            {primary && (
              <div className="text-[11px] text-stone-500">
                Default resolves to{' '}
                <span className="font-mono text-stone-700">
                  {primary.type === 'openhuman'
                    ? 'OpenHuman'
                    : `${primary.label} · ${primary.defaultModel}`}
                </span>
              </div>
            )}
          </section>
        </div>
        {/* end of Routing section */}
      </div>

      {isDirty && (
        <SaveBar
          diffSummary={diffSummary}
          changeCount={diffSummary.length}
          onSave={() => void save()}
          onDiscard={discard}
        />
      )}

      {editing && (
        <CloudProviderEditor
          initial={editing === 'new' ? null : editing}
          existingTypes={draft.cloudProviders
            .filter(p => p.id !== (editing === 'new' ? '' : editing.id))
            .map(p => p.type)}
          onClose={() => setEditing(null)}
          onSubmit={async (next, apiKey) => {
            setBusyAction('save-provider');
            try {
              const id =
                editing === 'new' || !editing.id
                  ? `p_${next.type}_${Math.random().toString(36).slice(2, 7)}`
                  : editing.id;
              const upserted: CloudProvider = {
                ...next,
                id,
                maskedKey: maskKeyLabel(apiKey ? true : next.maskedKey.startsWith('••••')),
              };
              const list =
                editing === 'new'
                  ? [...draft.cloudProviders, upserted]
                  : draft.cloudProviders.map(p => (p.id === editing.id ? upserted : p));
              setDraft({
                ...draft,
                cloudProviders: list,
                primaryCloudId: draft.primaryCloudId || upserted.id,
              });
              if (apiKey && upserted.type !== 'openhuman') {
                try {
                  await setCloudProviderKey(upserted.type as ApiCloudProviderType, apiKey);
                } catch (err) {
                  const msg = err instanceof Error ? err.message : String(err);

                  console.warn('[ai-settings] setCloudProviderKey failed', msg);
                }
              }
              setEditing(null);
            } finally {
              setBusyAction(null);
            }
          }}
          onClearKey={async type => {
            try {
              await clearCloudProviderKey(type as ApiCloudProviderType);
              await reload();
            } catch (err) {
              const msg = err instanceof Error ? err.message : String(err);

              console.warn('[ai-settings] clearCloudProviderKey failed', msg);
            }
          }}
        />
      )}

      {customDialogFor &&
        (() => {
          const w = WORKLOADS.find(x => x.id === customDialogFor);
          if (!w) return null;
          return (
            <CustomRoutingDialog
              workload={w}
              initial={draft.routing[customDialogFor]}
              cloudProviders={draft.cloudProviders}
              localModels={installed}
              ollamaRunning={ollama.state === 'running'}
              cloudModelIds={cloudModelIdsMap}
              onClose={() => setCustomDialogFor(null)}
              onSubmit={next => {
                updateRouting(customDialogFor, next);
                setCustomDialogFor(null);
              }}
            />
          );
        })()}

      {keyDialogFor && (
        <ProviderKeyDialog
          type={keyDialogFor}
          label={pendingLocalLabel ?? PROVIDER_META[keyDialogFor].label}
          onCancel={() => {
            setKeyDialogFor(null);
            setPendingLocalLabel(null);
          }}
          onSubmit={async (apiKey, modelIds) => {
            const type = keyDialogFor;
            const localLabel = pendingLocalLabel;
            setBusyAction(
              `toggle-${localLabel ? localLabel.toLowerCase().replace(/\s/g, '') : type}`
            );
            try {
              // For LM Studio / Ollama the dialog's "API key" field is
              // actually the local endpoint URL, so persist it as endpoint
              // and skip the credential save (no remote key to store).
              const isLocalRuntime = Boolean(localLabel);
              const upserted: CloudProvider = {
                id: `p_${type}_${Math.random().toString(36).slice(2, 7)}`,
                type,
                label: localLabel ?? PROVIDER_META[type].label,
                endpoint: isLocalRuntime ? apiKey.trim() : defaultEndpointFor(type),
                defaultModel: '',
                maskedKey: maskKeyLabel(true),
              };
              setDraft({ ...draft, cloudProviders: [...draft.cloudProviders, upserted] });
              if (!isLocalRuntime && type !== 'openhuman') {
                await setCloudProviderKey(type as ApiCloudProviderType, apiKey);
              }
              // Persist the model IDs so the custom-routing dropdown is
              // populated for this provider without needing the plaintext
              // key again. Best-effort.
              if (modelIds && modelIds.length > 0) {
                cacheProviderModelIds(type, modelIds);
              }
              setKeyDialogFor(null);
              setPendingLocalLabel(null);
            } finally {
              setBusyAction(null);
            }
          }}
        />
      )}
    </div>
  );
};

// ─────────────────────────────────────────────────────────────────────────────
// Cloud provider editor modal
// ─────────────────────────────────────────────────────────────────────────────

const CloudProviderEditor = ({
  initial,
  existingTypes,
  onClose,
  onSubmit,
  onClearKey,
}: {
  initial: CloudProvider | null;
  existingTypes: CloudProviderType[];
  onClose: () => void;
  onSubmit: (next: CloudProvider, apiKey: string) => Promise<void> | void;
  onClearKey: (type: CloudProviderType) => Promise<void> | void;
}) => {
  const defaultType: CloudProviderType =
    initial?.type ??
    (['openai', 'anthropic', 'openrouter', 'custom'] as CloudProviderType[]).find(
      t => !existingTypes.includes(t)
    ) ??
    'custom';
  const [type, setType] = useState<CloudProviderType>(defaultType);
  const [endpoint, setEndpoint] = useState(initial?.endpoint ?? defaultEndpointFor(defaultType));
  const [defaultModel, setDefaultModel] = useState(initial?.defaultModel ?? '');
  const [apiKey, setApiKey] = useState('');
  const [saving, setSaving] = useState(false);
  const isOpenHuman = type === 'openhuman';
  const hasExistingKey = (initial?.maskedKey ?? '').startsWith('••••');

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-stone-900/30 p-4">
      <div className="w-full max-w-md rounded-lg border border-stone-200 bg-white shadow-float">
        <div className="border-b border-stone-200 px-4 py-3">
          <div className="text-sm font-semibold text-stone-900">
            {initial ? `Edit ${initial.label}` : 'Add cloud provider'}
          </div>
          <div className="mt-0.5 text-xs text-stone-500">
            API keys are encrypted at rest in <span className="font-mono">auth-profiles.json</span>.
          </div>
        </div>
        <div className="space-y-3 px-4 py-3">
          <div>
            <label className="text-[10px] font-semibold uppercase tracking-wide text-stone-500">
              Provider
            </label>
            <select
              value={type}
              onChange={e => {
                const next = e.target.value as CloudProviderType;
                setType(next);
                if (!initial) {
                  setEndpoint(defaultEndpointFor(next));
                }
              }}
              disabled={!!initial}
              className="mt-1 w-full rounded-lg border border-stone-200 bg-white px-3 py-2 text-sm text-stone-900 disabled:opacity-60 focus:border-primary-400 focus:outline-none focus:ring-1 focus:ring-primary-200">
              {(['openai', 'anthropic', 'openrouter', 'custom'] as CloudProviderType[])
                .filter(t => t === type || !existingTypes.includes(t))
                .map(t => (
                  <option key={t} value={t}>
                    {PROVIDER_META[t].label}
                  </option>
                ))}
            </select>
          </div>
          <div>
            <label className="text-[10px] font-semibold uppercase tracking-wide text-stone-500">
              Endpoint
            </label>
            <input
              value={endpoint}
              onChange={e => setEndpoint(e.target.value)}
              disabled={isOpenHuman}
              className="mt-1 w-full rounded-lg border border-stone-200 bg-white px-3 py-2 font-mono text-xs text-stone-900 placeholder:text-stone-400 disabled:opacity-60 focus:border-primary-400 focus:outline-none focus:ring-1 focus:ring-primary-200"
              placeholder="https://api.example.com/v1"
            />
          </div>
          <div>
            <label className="text-[10px] font-semibold uppercase tracking-wide text-stone-500">
              Default model
            </label>
            <input
              value={defaultModel}
              onChange={e => setDefaultModel(e.target.value)}
              className="mt-1 w-full rounded-lg border border-stone-200 bg-white px-3 py-2 font-mono text-xs text-stone-900 placeholder:text-stone-400 focus:border-primary-400 focus:outline-none focus:ring-1 focus:ring-primary-200"
              placeholder="gpt-4o"
            />
          </div>
          {!isOpenHuman && (
            <div>
              <label className="flex items-center justify-between text-[10px] font-semibold uppercase tracking-wide text-stone-500">
                <span>API key</span>
                {hasExistingKey && (
                  <button
                    onClick={() => void onClearKey(type)}
                    className="text-[10px] font-medium normal-case text-coral-600 hover:text-coral-700">
                    Clear stored key
                  </button>
                )}
              </label>
              <input
                type="password"
                value={apiKey}
                onChange={e => setApiKey(e.target.value)}
                className="mt-1 w-full rounded-lg border border-stone-200 bg-white px-3 py-2 font-mono text-xs text-stone-900 placeholder:text-stone-400 focus:border-primary-400 focus:outline-none focus:ring-1 focus:ring-primary-200"
                placeholder={hasExistingKey ? 'Leave blank to keep existing key' : 'sk-...'}
              />
            </div>
          )}
        </div>
        <div className="flex items-center justify-end gap-2 border-t border-stone-200 px-4 py-3">
          <button
            onClick={onClose}
            disabled={saving}
            className="rounded-lg border border-stone-200 px-3 py-1.5 text-xs font-medium text-stone-700 hover:bg-stone-50 disabled:opacity-50">
            Cancel
          </button>
          <button
            onClick={async () => {
              setSaving(true);
              try {
                await onSubmit(
                  {
                    id: initial?.id ?? '',
                    type,
                    label: PROVIDER_META[type].label,
                    endpoint: endpoint.trim(),
                    maskedKey: maskKeyLabel(hasExistingKey || apiKey.length > 0),
                    defaultModel: defaultModel.trim(),
                  },
                  apiKey.trim()
                );
              } finally {
                setSaving(false);
              }
            }}
            disabled={saving || !endpoint.trim()}
            className="rounded-lg bg-primary-500 px-3 py-1.5 text-xs font-medium text-white hover:bg-primary-600 disabled:opacity-50">
            {saving ? 'Saving…' : initial ? 'Save changes' : 'Add provider'}
          </button>
        </div>
      </div>
    </div>
  );
};

function defaultEndpointFor(t: CloudProviderType): string {
  switch (t) {
    case 'openhuman':
      return 'https://api.openhuman.ai/v1';
    case 'openai':
      return 'https://api.openai.com/v1';
    case 'anthropic':
      return 'https://api.anthropic.com/v1';
    case 'openrouter':
      return 'https://openrouter.ai/api/v1';
    case 'custom':
      return '';
  }
}

export default AIPanel;
