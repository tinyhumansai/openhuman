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
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { LuCheck, LuCircleAlert } from 'react-icons/lu';

import {
  type AISettings as ApiAISettings,
  type ProviderRef as ApiProviderRef,
  clearCloudProviderKey,
  type CloudProviderView,
  loadAISettings,
  loadLocalProviderSnapshot,
  type LocalProviderSnapshot,
  saveAISettings,
  setCloudProviderKey,
} from '../../../services/api/aiSettingsApi';
import type { AuthStyle } from '../../../utils/tauriCommands/config';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

type CloudProvider = {
  id: string;
  slug: string;
  label: string;
  endpoint: string;
  authStyle: AuthStyle;
  maskedKey: string;
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
  | { kind: 'openhuman' }
  | { kind: 'cloud'; providerSlug: string; model: string }
  | { kind: 'local'; model: string };

type Workload = { id: WorkloadId; group: WorkloadGroup; label: string; description: string };

type RoutingMap = Record<WorkloadId, ProviderRef>;

// ─────────────────────────────────────────────────────────────────────────────
// Static catalog
// ─────────────────────────────────────────────────────────────────────────────

// Slug-keyed display metadata for built-in provider slugs. Used only for
// chip rendering (label, tone). Custom providers use `provider.label` directly.
const BUILTIN_PROVIDER_META: Record<string, { tone: string; label: string }> = {
  openhuman: { label: 'OpenHuman', tone: 'bg-primary-50 ring-primary-200 text-primary-900' },
  openai: { label: 'OpenAI', tone: 'bg-emerald-50 ring-emerald-200 text-emerald-900' },
  anthropic: { label: 'Anthropic', tone: 'bg-orange-50 ring-orange-200 text-orange-900' },
  openrouter: { label: 'OpenRouter', tone: 'bg-slate-100 ring-slate-300 text-slate-900' },
  custom: { label: 'Custom', tone: 'bg-stone-100 ring-stone-300 text-stone-900' },
};

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

// TIER_PRESETS removed alongside the Local provider section.

// ─────────────────────────────────────────────────────────────────────────────
// API-adapter hooks
//
// The panel works in terms of `CloudProvider` (slug + maskedKey) and
// `ProviderRef` (slug-keyed). The wire format is identical — this layer
// just derives the `maskedKey` display string from `has_api_key`.
// ─────────────────────────────────────────────────────────────────────────────

type AISettings = { cloudProviders: CloudProvider[]; routing: RoutingMap };

const EMPTY_ROUTING: RoutingMap = {
  reasoning: { kind: 'openhuman' },
  agentic: { kind: 'openhuman' },
  coding: { kind: 'openhuman' },
  memory: { kind: 'openhuman' },
  embeddings: { kind: 'openhuman' },
  heartbeat: { kind: 'openhuman' },
  learning: { kind: 'openhuman' },
  subconscious: { kind: 'openhuman' },
};

const EMPTY_SETTINGS: AISettings = { cloudProviders: [], routing: EMPTY_ROUTING };

function maskKeyLabel(hasKey: boolean): string {
  return hasKey ? '•••• configured' : 'Not configured';
}

/**
 * Default auth style for a slug. Built-in slugs map to their known styles;
 * everything else (custom + third-party slugs the user types in) defaults
 * to bearer, matching the OpenAI-compatible majority.
 */
function authStyleForSlug(slug: string): AuthStyle {
  if (slug === 'openhuman') return 'openhuman_jwt';
  if (slug === 'anthropic') return 'anthropic';
  if (slug === 'lmstudio' || slug === 'ollama') return 'none';
  return 'bearer';
}

function toPanelProvider(p: CloudProviderView): CloudProvider {
  return {
    id: p.id,
    slug: p.slug,
    label: p.label,
    endpoint: p.endpoint,
    authStyle: p.auth_style,
    maskedKey: maskKeyLabel(p.has_api_key),
  };
}

function toPanelRoutingFromApi(api: ApiAISettings): { panel: AISettings } {
  const cloudProviders = api.cloudProviders.map(toPanelProvider);
  // ApiProviderRef and ProviderRef share the same shape — pass through directly.
  const liftRef = (r: ApiProviderRef): ProviderRef => r;
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
  return { panel: { cloudProviders, routing } };
}

function toApiSettings(panel: AISettings): ApiAISettings {
  return {
    cloudProviders: panel.cloudProviders.map(p => ({
      id: p.id,
      slug: p.slug,
      label: p.label,
      endpoint: p.endpoint,
      auth_style: p.authStyle,
      has_api_key: p.maskedKey.startsWith('••••'),
    })),
    routing: {
      reasoning: panel.routing.reasoning,
      agentic: panel.routing.agentic,
      coding: panel.routing.coding,
      memory: panel.routing.memory,
      embeddings: panel.routing.embeddings,
      heartbeat: panel.routing.heartbeat,
      learning: panel.routing.learning,
      subconscious: panel.routing.subconscious,
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
      const prevApi = toApiSettings(saved);
      const nextApi = toApiSettings(draft);
      await saveAISettings(prevApi, nextApi);
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

// Local-runtime chip slugs (Ollama / LM Studio) that aren't actual slugs in
// the cloud_providers list but need the same chip affordance.
type LocalChipSlug = 'lmstudio' | 'ollama';

// Tints per local-runtime chip slug.
const LOCAL_CHIP_TONE: Record<LocalChipSlug, string> = {
  lmstudio: 'bg-cyan-50 ring-cyan-200 text-cyan-900',
  ollama: 'bg-violet-50 ring-violet-200 text-violet-900',
};

const LOCAL_CHIP_LABEL: Record<LocalChipSlug, string> = { lmstudio: 'LM Studio', ollama: 'Ollama' };

function slugTone(slug: string): string {
  return BUILTIN_PROVIDER_META[slug]?.tone ?? 'bg-stone-100 ring-stone-300 text-stone-900';
}

const ProviderToggleChip = ({
  slug,
  label,
  enabled,
  busy,
  onToggle,
}: {
  slug: string;
  label: string;
  enabled: boolean;
  busy?: boolean;
  onToggle: () => void;
}) => {
  const tone = slugTone(slug);
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
        className={`relative inline-flex h-4 w-7 shrink-0 items-center rounded-full transition-colors disabled:cursor-wait disabled:opacity-60 ${enabled ? 'bg-primary-500' : 'bg-stone-300'}`}>
        <span
          aria-hidden
          className={`inline-block h-3 w-3 transform rounded-full bg-white shadow transition-transform ${enabled ? 'translate-x-3.5' : 'translate-x-0.5'}`}
        />
      </button>
    </div>
  );
};

// Minimal API-key dialog — shown when the user flips a provider toggle ON.
// No endpoint / model fields; default endpoint is derived from the slug and
// the model is left empty (the routing dialog picks the model per workload).
const ProviderKeyDialog = ({
  slug,
  label,
  onCancel,
  onSubmit,
}: {
  slug: string;
  label: string;
  onCancel: () => void;
  onSubmit: (apiKey: string) => Promise<void> | void;
}) => {
  const [apiKey, setApiKey] = useState('');
  const [phase, setPhase] = useState<'idle' | 'saving'>('idle');
  const [error, setError] = useState<string | null>(null);
  const busy = phase !== 'idle';

  const placeholder =
    slug === 'openai'
      ? 'sk-...'
      : slug === 'anthropic'
        ? 'sk-ant-...'
        : slug === 'openrouter'
          ? 'sk-or-...'
          : 'your-api-key';

  const handleSave = async () => {
    const trimmed = apiKey.trim();
    if (!trimmed) {
      setError('Please paste your API key to continue.');
      return;
    }
    setError(null);

    setPhase('saving');
    try {
      await onSubmit(trimmed);
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
            }}
            className="rounded-lg border border-stone-300 bg-white px-3 py-2 text-sm text-stone-900 placeholder-stone-400 focus:border-primary-500 focus:outline-none focus:ring-1 focus:ring-primary-500 disabled:opacity-60"
          />
          {error ? <p className="text-xs font-medium text-red-600">{error}</p> : null}
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
            {phase === 'saving' ? 'Saving…' : 'Save'}
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
  cloudProviders: CloudProvider[];
  localModels: OllamaModel[];
  ollamaState: OllamaState;
  onChange: (next: ProviderRef) => void;
};

const WorkloadRow = ({
  workload,
  ref_,
  cloudProviders,
  localModels,
  ollamaState,
  onChange,
  onCustomClick,
}: WorkloadRowProps & { onCustomClick: () => void }) => {
  const selectedCloud =
    ref_.kind === 'cloud' ? cloudProviders.find(c => c.slug === ref_.providerSlug) : undefined;

  const isDefault = ref_.kind === 'openhuman';

  let resolved: string;
  if (ref_.kind === 'openhuman') {
    resolved = 'OpenHuman (default)';
  } else if (ref_.kind === 'cloud') {
    if (!selectedCloud) resolved = `${ref_.providerSlug} · ${ref_.model}`;
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
          onClick={() => onChange({ kind: 'openhuman' })}
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
  onClose: () => void;
  onSubmit: (next: ProviderRef) => void;
}

type CustomDialogSource = { kind: 'cloud'; providerSlug: string } | { kind: 'local' };

const CustomRoutingDialog = ({
  workload,
  initial,
  cloudProviders,
  localModels,
  ollamaRunning,
  onClose,
  onSubmit,
}: CustomRoutingDialogProps) => {
  // Non-openhuman cloud providers + local-ollama (if available) are the
  // "Custom" options. OpenHuman is excluded — it's the Default path.
  const customCloud = cloudProviders.filter(p => p.slug !== 'openhuman');
  const localAvailable = ollamaRunning && localModels.length > 0;

  const initialSource: CustomDialogSource | null =
    initial.kind === 'cloud'
      ? { kind: 'cloud', providerSlug: initial.providerSlug }
      : initial.kind === 'local'
        ? { kind: 'local' }
        : customCloud[0]
          ? { kind: 'cloud', providerSlug: customCloud[0].slug }
          : localAvailable
            ? { kind: 'local' }
            : null;

  const [source, setSource] = useState<CustomDialogSource | null>(initialSource);
  const [model, setModel] = useState<string>(() => {
    if (initial.kind === 'cloud' || initial.kind === 'local') return initial.model;
    if (initialSource?.kind === 'cloud') {
      const p = customCloud.find(c => c.slug === initialSource.providerSlug);
      return p ? '' : '';
    }
    return localModels[0]?.id ?? '';
  });

  const selectedCloud =
    source?.kind === 'cloud' ? customCloud.find(c => c.slug === source.providerSlug) : undefined;

  const canSave = source !== null && model.trim().length > 0;

  const handleSave = () => {
    if (!source || !canSave) return;
    if (source.kind === 'cloud') {
      onSubmit({ kind: 'cloud', providerSlug: source.providerSlug, model: model.trim() });
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
                  source
                    ? `${source.kind}:${source.kind === 'cloud' ? source.providerSlug : ''}`
                    : ''
                }
                onChange={e => {
                  const colonIdx = e.target.value.indexOf(':');
                  const kind = e.target.value.slice(0, colonIdx);
                  const slug = e.target.value.slice(colonIdx + 1);
                  if (kind === 'local') {
                    setSource({ kind: 'local' });
                    setModel(localModels[0]?.id ?? '');
                  } else if (kind === 'cloud') {
                    setSource({ kind: 'cloud', providerSlug: slug });
                    setModel('');
                  }
                }}
                className="rounded-lg border border-stone-300 bg-white px-3 py-2 text-sm text-stone-900 focus:border-primary-500 focus:outline-none focus:ring-1 focus:ring-primary-500">
                {customCloud.map(p => (
                  <option key={p.slug} value={`cloud:${p.slug}`}>
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
                <input
                  type="text"
                  value={model}
                  onChange={e => setModel(e.target.value)}
                  placeholder={selectedCloud ? `${selectedCloud.slug} model id` : 'model-id'}
                  className="rounded-lg border border-stone-300 bg-white px-3 py-2 text-sm font-mono text-stone-900 placeholder-stone-400 focus:border-primary-500 focus:outline-none focus:ring-1 focus:ring-primary-500"
                />
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
  // Which provider slug's API-key dialog is currently open (null = closed).
  const [keyDialogFor, setKeyDialogFor] = useState<string | null>(null);
  // When the user toggles LM Studio / Ollama (local runtimes), we
  // need to remember which label to attach to the upserted provider so the
  // chip can find it again. Cleared when the dialog closes.
  const [pendingLocalLabel, setPendingLocalLabel] = useState<string | null>(null);

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
          r.kind === 'openhuman'
            ? 'openhuman'
            : r.kind === 'cloud'
              ? `${r.providerSlug}:${r.model}`
              : `local:${r.model}`;
        out.push(`${w.label} → ${describe(b)}`);
      }
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
              {/* Built-in cloud providers — openai/anthropic/openrouter/custom */}
              {(['openai', 'anthropic', 'openrouter', 'custom'] as const).map(slug => {
                const meta = BUILTIN_PROVIDER_META[slug];
                const label = meta?.label ?? slug;
                const existing = draft.cloudProviders.find(cp => cp.slug === slug);
                const enabled = !!existing;
                return (
                  <ProviderToggleChip
                    key={slug}
                    slug={slug}
                    label={label}
                    enabled={enabled}
                    busy={busyAction === `toggle-${slug}`}
                    onToggle={() => {
                      if (enabled && existing) {
                        // Toggle OFF: remove the provider + scrub any
                        // routing entries that pin to it.
                        const remaining = draft.cloudProviders.filter(cp => cp.id !== existing.id);
                        const nextRouting = Object.fromEntries(
                          Object.entries(draft.routing).map(([wid, ref]) => [
                            wid,
                            ref.kind === 'cloud' && ref.providerSlug === existing.slug
                              ? ({ kind: 'openhuman' } as const)
                              : ref,
                          ])
                        ) as typeof draft.routing;
                        setDraft({ ...draft, cloudProviders: remaining, routing: nextRouting });
                      } else if (slug === 'custom') {
                        // Custom providers need slug + endpoint + label, not
                        // just an API key — defer to the full editor modal.
                        setEditing('new');
                      } else {
                        // Toggle ON: open the API-key popup. The chip
                        // only flips after the dialog saves.
                        setKeyDialogFor(slug);
                      }
                    }}
                  />
                );
              })}

              {/* LM Studio + Ollama — local runtimes stored with a slug of
                  "lmstudio" / "ollama" so they're distinct from generic custom. */}
              {(['lmstudio', 'ollama'] as const).map(localKind => {
                const label = LOCAL_CHIP_LABEL[localKind];
                const tone = LOCAL_CHIP_TONE[localKind];
                const existing = draft.cloudProviders.find(cp => cp.slug === localKind);
                const enabled = !!existing;
                // Use a styled chip directly for local runtimes — they have
                // non-standard tones not in BUILTIN_PROVIDER_META.
                return (
                  <div
                    key={localKind}
                    className={`inline-flex items-center gap-2 rounded-full px-2.5 py-1 text-xs font-medium ring-1 transition-colors ${tone}`}>
                    <span>{label}</span>
                    <button
                      type="button"
                      role="switch"
                      aria-checked={enabled}
                      aria-label={`${enabled ? 'Disconnect' : 'Connect'} ${label}`}
                      disabled={busyAction === `toggle-${localKind}`}
                      onClick={() => {
                        if (enabled && existing) {
                          const remaining = draft.cloudProviders.filter(
                            cp => cp.id !== existing.id
                          );
                          const nextRouting = Object.fromEntries(
                            Object.entries(draft.routing).map(([wid, ref]) => [
                              wid,
                              ref.kind === 'cloud' && ref.providerSlug === localKind
                                ? ({ kind: 'openhuman' } as const)
                                : ref,
                            ])
                          ) as typeof draft.routing;
                          setDraft({ ...draft, cloudProviders: remaining, routing: nextRouting });
                        } else {
                          setKeyDialogFor(localKind);
                          setPendingLocalLabel(label);
                        }
                      }}
                      className={`relative inline-flex h-4 w-7 shrink-0 items-center rounded-full transition-colors disabled:cursor-wait disabled:opacity-60 ${enabled ? 'bg-primary-500' : 'bg-stone-300'}`}>
                      <span
                        aria-hidden
                        className={`inline-block h-3 w-3 transform rounded-full bg-white shadow transition-transform ${enabled ? 'translate-x-3.5' : 'translate-x-0.5'}`}
                      />
                    </button>
                  </div>
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

            <div className="text-[11px] text-stone-500">
              Default resolves to <span className="font-mono text-stone-700">OpenHuman</span>.
            </div>
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
          existingSlugs={draft.cloudProviders
            .filter(p => p.id !== (editing === 'new' ? '' : editing.id))
            .map(p => p.slug)}
          onClose={() => setEditing(null)}
          onSubmit={async (next, apiKey) => {
            setBusyAction('save-provider');
            try {
              const id =
                editing === 'new' || !editing.id
                  ? `p_${next.slug}_${Math.random().toString(36).slice(2, 7)}`
                  : editing.id;
              const upserted: CloudProvider = {
                ...next,
                id,
                maskedKey: maskKeyLabel(apiKey ? true : next.maskedKey.startsWith('••••')),
              };
              // Persist the credential BEFORE mutating draft, so a key-write
              // failure doesn't leave the config referencing a provider with
              // no stored key.
              if (apiKey && upserted.slug !== 'openhuman') {
                try {
                  await setCloudProviderKey(upserted.slug, apiKey);
                } catch (err) {
                  const msg = err instanceof Error ? err.message : String(err);
                  console.warn('[ai-settings] setCloudProviderKey failed', msg);
                  return;
                }
              }
              const list =
                editing === 'new'
                  ? [...draft.cloudProviders, upserted]
                  : draft.cloudProviders.map(p => (p.id === editing.id ? upserted : p));
              setDraft({ ...draft, cloudProviders: list });
              setEditing(null);
            } finally {
              setBusyAction(null);
            }
          }}
          onClearKey={async slug => {
            try {
              await clearCloudProviderKey(slug);
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
          slug={keyDialogFor}
          label={pendingLocalLabel ?? BUILTIN_PROVIDER_META[keyDialogFor]?.label ?? keyDialogFor}
          onCancel={() => {
            setKeyDialogFor(null);
            setPendingLocalLabel(null);
          }}
          onSubmit={async apiKey => {
            const slug = keyDialogFor;
            const localLabel = pendingLocalLabel;
            setBusyAction(
              `toggle-${localLabel ? localLabel.toLowerCase().replace(/\s/g, '') : slug}`
            );
            try {
              // For LM Studio / Ollama the dialog's "API key" field is
              // actually the local endpoint URL, so persist it as endpoint
              // and skip the credential save (no remote key to store).
              const isLocalRuntime = Boolean(localLabel);
              const upserted: CloudProvider = {
                id: `p_${slug}_${Math.random().toString(36).slice(2, 7)}`,
                slug,
                label: localLabel ?? BUILTIN_PROVIDER_META[slug]?.label ?? slug,
                endpoint: isLocalRuntime ? apiKey.trim() : defaultEndpointFor(slug),
                authStyle: authStyleForSlug(slug),
                maskedKey: maskKeyLabel(true),
              };
              // Persist the credential BEFORE mutating draft, so a key-write
              // failure can't leave config + secrets out of sync.
              if (!isLocalRuntime && slug !== 'openhuman') {
                try {
                  await setCloudProviderKey(slug, apiKey);
                } catch (err) {
                  const msg = err instanceof Error ? err.message : String(err);
                  console.warn('[ai-settings] setCloudProviderKey failed', msg);
                  return;
                }
              }
              setDraft({ ...draft, cloudProviders: [...draft.cloudProviders, upserted] });
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
  existingSlugs,
  onClose,
  onSubmit,
  onClearKey,
}: {
  initial: CloudProvider | null;
  existingSlugs: string[];
  onClose: () => void;
  onSubmit: (next: CloudProvider, apiKey: string) => Promise<void> | void;
  onClearKey: (slug: string) => Promise<void> | void;
}) => {
  const defaultSlug: string =
    initial?.slug ??
    (['openai', 'anthropic', 'openrouter', 'custom'] as const).find(
      s => !existingSlugs.includes(s)
    ) ??
    'custom';
  const [slug, setSlug] = useState<string>(defaultSlug);
  const [label, setLabel] = useState<string>(
    initial?.label ?? BUILTIN_PROVIDER_META[defaultSlug]?.label ?? defaultSlug
  );
  const [endpoint, setEndpoint] = useState(initial?.endpoint ?? defaultEndpointFor(defaultSlug));
  const [apiKey, setApiKey] = useState('');
  const [saving, setSaving] = useState(false);
  const isOpenHuman = slug === 'openhuman';
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
              Provider slug
            </label>
            <select
              value={slug}
              onChange={e => {
                const next = e.target.value;
                setSlug(next);
                setLabel(BUILTIN_PROVIDER_META[next]?.label ?? next);
                if (!initial) {
                  setEndpoint(defaultEndpointFor(next));
                }
              }}
              disabled={!!initial}
              className="mt-1 w-full rounded-lg border border-stone-200 bg-white px-3 py-2 text-sm text-stone-900 disabled:opacity-60 focus:border-primary-400 focus:outline-none focus:ring-1 focus:ring-primary-200">
              {(['openai', 'anthropic', 'openrouter', 'custom'] as const)
                .filter(s => s === slug || !existingSlugs.includes(s))
                .map(s => (
                  <option key={s} value={s}>
                    {BUILTIN_PROVIDER_META[s]?.label ?? s}
                  </option>
                ))}
            </select>
          </div>
          <div>
            <label className="text-[10px] font-semibold uppercase tracking-wide text-stone-500">
              Display label
            </label>
            <input
              value={label}
              onChange={e => setLabel(e.target.value)}
              className="mt-1 w-full rounded-lg border border-stone-200 bg-white px-3 py-2 text-sm text-stone-900 placeholder:text-stone-400 focus:border-primary-400 focus:outline-none focus:ring-1 focus:ring-primary-200"
              placeholder="My Provider"
            />
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
          {!isOpenHuman && (
            <div>
              <label className="flex items-center justify-between text-[10px] font-semibold uppercase tracking-wide text-stone-500">
                <span>API key</span>
                {hasExistingKey && (
                  <button
                    onClick={() => void onClearKey(slug)}
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
                    slug,
                    label: label.trim() || slug,
                    endpoint: endpoint.trim(),
                    authStyle: initial?.authStyle ?? authStyleForSlug(slug),
                    maskedKey: maskKeyLabel(hasExistingKey || apiKey.length > 0),
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

function defaultEndpointFor(slug: string): string {
  switch (slug) {
    case 'openhuman':
      return 'https://api.openhuman.ai/v1';
    case 'openai':
      return 'https://api.openai.com/v1';
    case 'anthropic':
      return 'https://api.anthropic.com/v1';
    case 'openrouter':
      return 'https://openrouter.ai/api/v1';
    default:
      return '';
  }
}

export default AIPanel;
