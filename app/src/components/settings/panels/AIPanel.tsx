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
import {
  LuCheck,
  LuCircleAlert,
  LuCloud,
  LuCpu,
  LuDownload,
  LuKey,
  LuLoader,
  LuPencilLine,
  LuPlus,
  LuPower,
  LuRefreshCw,
  LuServer,
  LuShield,
  LuTrash2,
  LuWand,
  LuZap,
} from 'react-icons/lu';

import {
  type AISettings as ApiAISettings,
  type ProviderRef as ApiProviderRef,
  clearCloudProviderKey,
  type CloudProviderView,
  loadAISettings,
  loadLocalProviderSnapshot,
  localProvider,
  type LocalProviderSnapshot,
  saveAISettings,
  setCloudProviderKey,
} from '../../../services/api/aiSettingsApi';
import { openUrl } from '../../../utils/openUrl';
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

const TIER_PRESETS = [
  { id: 'lite', label: '2–4 GB RAM', model: 'llama3.2:1b', blurb: 'Tiny + responsive' },
  { id: 'standard', label: '4–8 GB RAM', model: 'llama3.1:8b', blurb: 'Balanced default' },
  { id: 'studio', label: '8 GB+ RAM', model: 'qwen2.5:14b', blurb: 'Headroom for nuance' },
];

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

const SectionLabel = ({ children }: { children: React.ReactNode }) => (
  <h3 className="text-xs font-semibold uppercase tracking-wide text-stone-500">{children}</h3>
);

const formatBytes = (n: number): string => {
  if (n < 1024 ** 2) return `${(n / 1024).toFixed(0)} KB`;
  if (n < 1024 ** 3) return `${(n / 1024 ** 2).toFixed(0)} MB`;
  return `${(n / 1024 ** 3).toFixed(1)} GB`;
};

const StatusDot = ({ state }: { state: OllamaState }) => {
  const tone =
    state === 'running'
      ? 'bg-sage-500'
      : state === 'starting'
        ? 'bg-amber-500'
        : state === 'error'
          ? 'bg-coral-500'
          : 'bg-stone-300';
  return (
    <span className="relative inline-flex h-2 w-2 items-center justify-center">
      <span
        className={`absolute inset-0 rounded-full ${tone} ${state === 'running' ? 'animate-glow-pulse' : ''}`}
      />
    </span>
  );
};

const ProviderChip = ({ type }: { type: CloudProviderType }) => {
  const meta = PROVIDER_META[type];
  return (
    <span
      className={`inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-[10px] font-medium ring-1 ring-inset ${meta.pill}`}>
      {meta.icon}
      {meta.label}
    </span>
  );
};

// ─────────────────────────────────────────────────────────────────────────────
// Cloud provider card
// ─────────────────────────────────────────────────────────────────────────────

const CloudProviderCard = ({
  provider,
  isPrimary,
  onMakePrimary,
  onEdit,
  onRemove,
}: {
  provider: CloudProvider;
  isPrimary: boolean;
  onMakePrimary: () => void;
  onEdit: () => void;
  onRemove: () => void;
}) => {
  const meta = PROVIDER_META[provider.type];
  return (
    <div
      className={`group relative flex overflow-hidden rounded-lg border ${
        isPrimary
          ? 'border-primary-300 bg-primary-50/30 ring-1 ring-primary-100'
          : 'border-stone-200 bg-stone-50'
      }`}>
      <div className={`w-0.5 shrink-0 ${meta.rail}`} aria-hidden />
      <div className="flex flex-1 flex-col gap-2 p-3">
        <div className="flex items-start justify-between gap-2">
          <div className="flex flex-wrap items-center gap-1.5">
            <span className="text-sm font-semibold text-stone-900">{provider.label}</span>
            {isPrimary && (
              <span className="rounded bg-primary-500 px-1.5 py-0.5 font-mono text-[9px] font-medium uppercase tracking-widest text-white">
                Primary
              </span>
            )}
            <ProviderChip type={provider.type} />
          </div>
          <div className="flex shrink-0 items-center gap-0.5">
            {!isPrimary && (
              <button
                onClick={onMakePrimary}
                className="rounded px-1.5 py-0.5 text-[11px] font-medium text-primary-600 hover:bg-primary-100/60">
                Set primary
              </button>
            )}
            {/* OpenHuman is the signed-in default: its endpoint comes from
                the user's account, its key is the session JWT (managed
                separately), and its type can't be changed. So edit + delete
                are both meaningless here — hide them to avoid confusion.
                "Set primary" stays, so the user can still re-mark OpenHuman
                as primary if they've switched to another provider. */}
            {provider.type !== 'openhuman' && (
              <>
                <button
                  onClick={onEdit}
                  className="rounded p-1 text-stone-400 hover:bg-stone-100 hover:text-stone-700"
                  aria-label="Edit">
                  <LuPencilLine className="h-3 w-3" />
                </button>
                <button
                  onClick={onRemove}
                  className="rounded p-1 text-stone-400 hover:bg-coral-50 hover:text-coral-600"
                  aria-label="Remove">
                  <LuTrash2 className="h-3 w-3" />
                </button>
              </>
            )}
          </div>
        </div>
        {provider.type === 'openhuman' ? (
          <div className="text-xs text-stone-500">Signed-in default · no configuration needed</div>
        ) : (
          <dl className="grid grid-cols-[auto_1fr] gap-x-2 gap-y-1 text-[11px]">
            <dt className="text-stone-400">Endpoint</dt>
            <dd className="truncate font-mono text-stone-700">{provider.endpoint}</dd>
            <dt className="text-stone-400">Key</dt>
            <dd className="flex items-center gap-1 truncate font-mono text-stone-700">
              <LuKey className="h-2.5 w-2.5 text-stone-400" />
              <span className="truncate">{provider.maskedKey}</span>
            </dd>
            <dt className="text-stone-400">Model</dt>
            <dd className="truncate font-mono text-stone-700">{provider.defaultModel}</dd>
          </dl>
        )}
      </div>
    </div>
  );
};

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
}: WorkloadRowProps) => {
  const localAvailable = ollamaState === 'running' && localModels.length > 0;
  const selectedCloud =
    ref_.kind === 'cloud' ? cloudProviders.find(c => c.id === ref_.providerId) : undefined;

  const tabBase = 'flex-1 px-2 py-1 text-[11px] font-medium transition-colors';
  const tab = (active: boolean, disabled = false) =>
    `${tabBase} first:rounded-l last:rounded-r ${
      active
        ? 'bg-white text-stone-900 shadow-subtle ring-1 ring-stone-200'
        : disabled
          ? 'text-stone-300'
          : 'text-stone-500 hover:text-stone-800'
    } ${disabled ? 'cursor-not-allowed' : 'cursor-pointer'}`;

  let resolved: string;
  if (ref_.kind === 'primary') {
    if (!primary) resolved = 'no primary set';
    else if (primary.type === 'openhuman') resolved = 'openhuman';
    else resolved = `${PROVIDER_META[primary.type].label.toLowerCase()} · ${primary.defaultModel}`;
  } else if (ref_.kind === 'cloud') {
    if (!selectedCloud) resolved = ref_.model;
    else if (selectedCloud.type === 'openhuman') resolved = 'openhuman';
    else resolved = `${PROVIDER_META[selectedCloud.type].label.toLowerCase()} · ${ref_.model}`;
  } else {
    resolved = `ollama · ${ref_.model}`;
  }

  return (
    <div className="space-y-2 py-2.5">
      <div className="flex items-start justify-between gap-2">
        <div className="min-w-0">
          <div className="text-sm font-medium text-stone-900">{workload.label}</div>
          <div className="truncate text-xs text-stone-500">{workload.description}</div>
        </div>
        <div className="inline-flex shrink-0 items-center rounded bg-stone-100 p-0.5">
          <button
            onClick={() => onChange({ kind: 'primary' })}
            className={tab(ref_.kind === 'primary')}>
            Primary
          </button>
          <button
            onClick={() => {
              const p = cloudProviders.find(c => c.id !== primary?.id) ?? cloudProviders[0];
              if (!p) return;
              onChange({
                kind: 'cloud',
                providerId: p.id,
                model: p.type === 'openhuman' ? '' : p.defaultModel,
              });
            }}
            className={tab(ref_.kind === 'cloud')}>
            Cloud
          </button>
          <button
            onClick={() => {
              if (!localAvailable) return;
              onChange({ kind: 'local', model: localModels[0]?.id ?? '' });
            }}
            className={tab(ref_.kind === 'local', !localAvailable)}
            title={!localAvailable ? 'Ollama not running' : undefined}>
            Local
          </button>
        </div>
      </div>

      {ref_.kind === 'primary' && (
        <div className="text-right font-mono text-[11px] text-stone-400">↳ {resolved}</div>
      )}
      {ref_.kind === 'cloud' && (
        <div className="flex items-center justify-end gap-1.5">
          <select
            value={ref_.providerId}
            onChange={e => {
              const p = cloudProviders.find(c => c.id === e.target.value)!;
              onChange({
                kind: 'cloud',
                providerId: p.id,
                model: p.type === 'openhuman' ? '' : p.defaultModel,
              });
            }}
            className="rounded-md border border-stone-300 bg-white px-2 py-1 font-mono text-[11px] text-stone-800 focus:border-primary-400 focus:outline-none focus:ring-1 focus:ring-primary-200">
            {cloudProviders.map(p => (
              <option key={p.id} value={p.id}>
                {PROVIDER_META[p.type].label}
              </option>
            ))}
          </select>
          {selectedCloud?.type !== 'openhuman' && (
            <input
              value={ref_.model}
              onChange={e =>
                onChange({ kind: 'cloud', providerId: ref_.providerId, model: e.target.value })
              }
              className="w-28 rounded-md border border-stone-300 bg-white px-2 py-1 font-mono text-[11px] text-stone-800 focus:border-primary-400 focus:outline-none focus:ring-1 focus:ring-primary-200"
            />
          )}
        </div>
      )}
      {ref_.kind === 'local' && (
        <div className="flex justify-end">
          <select
            value={ref_.model}
            onChange={e => onChange({ kind: 'local', model: e.target.value })}
            className="rounded-md border border-stone-300 bg-white px-2 py-1 font-mono text-[11px] text-stone-800 focus:border-primary-400 focus:outline-none focus:ring-1 focus:ring-primary-200">
            {localModels.map(m => (
              <option key={m.id} value={m.id}>
                {m.id}
              </option>
            ))}
          </select>
        </div>
      )}
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

const AIPanel = () => {
  const { navigateBack, breadcrumbs } = useSettingsNavigation();
  const { saved, draft, setDraft, isDirty, save, discard, loading, error, reload } =
    useAISettings();
  const ollama = useOllamaStatus();
  const installed = useInstalledModels(ollama.snapshot);
  const [editing, setEditing] = useState<CloudProvider | 'new' | null>(null);
  const [busyAction, setBusyAction] = useState<string | null>(null);
  const [advancedOpen, setAdvancedOpen] = useState<boolean>(false);
  const [customPathInput, setCustomPathInput] = useState<string>('');
  // Seed the custom-path input from the resolved binary path the FIRST time
  // diagnostics arrives, so the field shows what's currently in use.
  const resolvedBinaryPath = ollama.snapshot?.diagnostics?.ollama_binary_path ?? '';
  useEffect(() => {
    if (customPathInput === '' && resolvedBinaryPath) {
      setCustomPathInput(resolvedBinaryPath);
    }
    // We deliberately do NOT re-sync on every diagnostics tick — that would
    // clobber in-flight edits while the user is typing.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [resolvedBinaryPath]);

  const daemonWarning = ollama.snapshot?.status?.warning ?? '';
  const isDaemonConflict =
    daemonWarning.toLowerCase().includes('external ollama daemon') ||
    daemonWarning.toLowerCase().includes('broken runner');

  const primary = useMemo(
    () => draft.cloudProviders.find(p => p.id === draft.primaryCloudId),
    [draft]
  );

  const updateRouting = (id: WorkloadId, next: ProviderRef) =>
    setDraft({ ...draft, routing: { ...draft.routing, [id]: next } });

  const applyPreset = (kind: 'cloud' | 'local' | 'mixed') => {
    const next: RoutingMap = { ...draft.routing };
    for (const w of WORKLOADS) {
      if (kind === 'cloud') next[w.id] = { kind: 'primary' };
      else if (kind === 'local') {
        const m = installed[0]?.id;
        next[w.id] = m ? { kind: 'local', model: m } : { kind: 'primary' };
      } else {
        const firstModel = installed[0]?.id;
        next[w.id] =
          w.group === 'chat' || !firstModel
            ? { kind: 'primary' }
            : { kind: 'local', model: firstModel };
      }
    }
    setDraft({ ...draft, routing: next });
  };

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
      out.push(`primary → ${p ? PROVIDER_META[p.type].label : '—'}`);
    }
    return out;
  }, [saved, draft]);

  const chatRows = WORKLOADS.filter(w => w.group === 'chat');
  const bgRows = WORKLOADS.filter(w => w.group === 'background');

  return (
    <div className="relative">
      <SettingsHeader title="LLM" showBackButton onBack={navigateBack} breadcrumbs={breadcrumbs} />

      <div className="space-y-4 p-4">
        {/* ─── Cloud providers ─────────────────────────────────────────── */}
        <section className="space-y-3">
          <div className="flex items-center justify-between">
            <SectionLabel>Cloud providers</SectionLabel>
            <button
              onClick={() => setEditing('new')}
              className="inline-flex items-center gap-1 rounded-md border border-stone-200 px-2 py-1 text-xs font-medium text-stone-700 hover:border-primary-300 hover:bg-primary-50/40 hover:text-primary-700">
              <LuPlus className="h-3 w-3" />
              Add
            </button>
          </div>

          {loading && <div className="text-xs text-stone-500">Loading…</div>}
          {error && (
            <div className="rounded-md border border-coral-200 bg-coral-50 px-3 py-2 text-xs text-coral-700">
              {error}
            </div>
          )}

          <div className="space-y-2">
            {draft.cloudProviders.map(p => (
              <CloudProviderCard
                key={p.id}
                provider={p}
                isPrimary={p.id === draft.primaryCloudId}
                onMakePrimary={() => setDraft({ ...draft, primaryCloudId: p.id })}
                onEdit={() => setEditing(p)}
                onRemove={() => {
                  const remaining = draft.cloudProviders.filter(cp => cp.id !== p.id);
                  // If the removed provider was primary, clear or reassign.
                  const nextPrimaryId =
                    draft.primaryCloudId === p.id
                      ? (remaining[0]?.id ?? null)
                      : draft.primaryCloudId;
                  // Scrub pinned workload routes that reference the removed provider.
                  const nextRouting = Object.fromEntries(
                    Object.entries(draft.routing).map(([wid, ref]) => [
                      wid,
                      ref.kind === 'cloud' && ref.providerId === p.id
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
                }}
              />
            ))}
          </div>
        </section>

        {/* ─── Local provider ──────────────────────────────────────────── */}
        <section className="space-y-3">
          <SectionLabel>Local provider</SectionLabel>

          {/* Master enable / disable for the Ollama runtime.
              The `state` field is the source of truth: when
              `runtime_enabled = false` in config, bootstrap forces
              status.state = "disabled". Flipping ON also has to kick
              `local_ai_download` because bootstrap only auto-fires from
              "idle"/"degraded" — "disabled" → "ready" is NOT an automatic
              transition. */}
          <label
            className={`flex items-start gap-3 rounded-lg border border-stone-200 bg-white px-3 py-2.5 transition-opacity ${
              busyAction === 'toggle-local' ? 'cursor-wait opacity-80' : 'cursor-pointer'
            }`}>
            {busyAction === 'toggle-local' ? (
              <span className="mt-0.5 flex h-3.5 w-3.5 items-center justify-center">
                <LuLoader className="h-3.5 w-3.5 animate-spin text-primary-500" />
              </span>
            ) : (
              <input
                type="checkbox"
                checked={ollama.state !== 'disabled'}
                onChange={async e => {
                  const next = e.target.checked;
                  setBusyAction('toggle-local');
                  try {
                    if (next) {
                      // Enable: write config, then kick reset_to_idle +
                      // bootstrap. Bootstrap brings the daemon up (or sees
                      // an external one and leaves it alone).
                      await localProvider.setEnabled(true);
                      await localProvider.download(true);
                    } else {
                      // Disable as a GATE, not a process murder. The
                      // backend writes runtime_enabled=false, kills the
                      // daemon ONLY if OpenHuman spawned it (external
                      // installations stay running, per the same
                      // friendly-fire-avoidance principle used at startup),
                      // and forces status to "disabled" so the UI reflects
                      // the gated state immediately. From the factory's
                      // perspective the result is identical: any workload
                      // routed to `ollama:<model>` fails at build time.
                      await localProvider.shutdown();
                    }
                    // Poll every 500ms (up to 10s) until status settles.
                    // Disable usually finishes in <1s (the RPC marks
                    // status directly); enable can take 2-8s for daemon
                    // boot + initial model probe.
                    const startedAt = Date.now();
                    const targetReached = (s: string) =>
                      next ? Boolean(s) && s !== 'disabled' : s === 'disabled';
                    while (Date.now() - startedAt < 10_000) {
                      await new Promise(r => setTimeout(r, 500));
                      const fresh = await ollama.refresh();
                      const freshState = fresh?.status?.state ?? '';
                      if (targetReached(freshState)) break;
                    }
                  } catch (err) {
                    const msg = err instanceof Error ? err.message : String(err);
                    // eslint-disable-next-line no-console
                    console.warn('[ai-settings] toggle local AI failed', msg);
                  } finally {
                    setBusyAction(null);
                    await ollama.refresh();
                    await reload();
                  }
                }}
                className="mt-0.5"
              />
            )}
            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-2">
                <span className="text-sm font-medium text-stone-900">Enable local AI (Ollama)</span>
                {busyAction === 'toggle-local' && (
                  <span className="text-[10px] font-medium uppercase tracking-widest text-primary-500">
                    {ollama.state === 'disabled' ? 'Starting…' : 'Stopping…'}
                  </span>
                )}
              </div>
              <div className="text-xs text-stone-500">
                Manages the on-device Ollama daemon used by any workload routed to
                <span className="font-mono"> ollama:&lt;model&gt;</span>. Turn off if you only use
                cloud providers — saves CPU + RAM.
              </div>
            </div>
          </label>

          <div
            className={`overflow-hidden rounded-lg border border-stone-200 bg-stone-50 ${
              ollama.state === 'disabled' ? 'opacity-60' : ''
            }`}>
            <div className="flex items-center gap-2 border-b border-stone-200 px-3 py-2.5">
              <StatusDot state={ollama.state} />
              <div className="min-w-0 flex-1">
                <div className="text-sm font-medium capitalize text-stone-900">{ollama.state}</div>
                <div className="font-mono text-[10px] text-stone-400">
                  ollama{ollama.version ? ` · ${ollama.version}` : ''}
                </div>
                {ollama.snapshot?.status?.warning && (
                  <div className="mt-0.5 text-[10px] text-amber-700">
                    {ollama.snapshot.status.warning}
                  </div>
                )}
                {typeof ollama.snapshot?.status?.download_progress === 'number' && (
                  <div className="mt-0.5 text-[10px] text-stone-500">
                    Download {(ollama.snapshot.status.download_progress * 100).toFixed(0)}%
                  </div>
                )}
              </div>
              <button
                onClick={async () => {
                  setBusyAction('download');
                  try {
                    await localProvider.download(true);
                  } finally {
                    setBusyAction(null);
                    await ollama.refresh();
                  }
                }}
                disabled={busyAction === 'download' || ollama.state === 'disabled'}
                className="inline-flex items-center gap-1 rounded-md border border-stone-200 bg-white px-2 py-1 text-xs font-medium text-stone-700 hover:bg-stone-50 disabled:opacity-50"
                title={ollama.state === 'disabled' ? 'Enable local AI above first' : undefined}>
                {busyAction === 'download' ? (
                  <LuLoader className="h-3 w-3 animate-spin" />
                ) : (
                  <LuPower className="h-3 w-3" />
                )}
                {busyAction === 'download'
                  ? 'Working…'
                  : ollama.state === 'missing'
                    ? 'Install'
                    : 'Retry'}
              </button>
              <button
                onClick={() => void ollama.refresh()}
                className="inline-flex items-center gap-1 rounded-md border border-stone-200 bg-white px-2 py-1 text-xs font-medium text-stone-700 hover:bg-stone-50">
                <LuRefreshCw className="h-3 w-3" />
                Refresh
              </button>
            </div>

            {installed.length === 0 ? (
              <div className="p-3">
                <p className="text-xs text-stone-600">
                  {ollama.snapshot?.presets?.recommended_tier
                    ? `Recommended for this device: ${ollama.snapshot.presets.recommended_tier}.`
                    : 'Pick a tier preset to install a default model.'}
                </p>
                <div className="mt-2 space-y-1.5">
                  {(ollama.snapshot?.presets?.presets ?? []).map(t => (
                    <button
                      key={t.tier}
                      onClick={async () => {
                        setBusyAction(`preset:${t.tier}`);
                        try {
                          await localProvider.applyPreset(t.tier);
                          await reload();
                        } finally {
                          setBusyAction(null);
                          await ollama.refresh();
                        }
                      }}
                      disabled={!!busyAction}
                      className="flex w-full items-center justify-between rounded-md border border-stone-200 bg-white px-2.5 py-2 text-left hover:border-primary-300 hover:bg-primary-50/30 disabled:opacity-50">
                      <div>
                        <div className="text-[10px] font-semibold uppercase tracking-wide text-stone-500">
                          {t.label}
                        </div>
                        <div className="font-mono text-xs text-stone-900">{t.chat_model_id}</div>
                      </div>
                      <span className="text-[10px] text-stone-400">{t.description}</span>
                    </button>
                  ))}
                  {(ollama.snapshot?.presets?.presets ?? []).length === 0 &&
                    TIER_PRESETS.map(t => (
                      <div
                        key={t.id}
                        className="flex w-full items-center justify-between rounded-md border border-dashed border-stone-200 bg-white/60 px-2.5 py-2 text-left opacity-70">
                        <div>
                          <div className="text-[10px] font-semibold uppercase tracking-wide text-stone-500">
                            {t.label}
                          </div>
                          <div className="font-mono text-xs text-stone-900">{t.model}</div>
                        </div>
                        <span className="text-[10px] text-stone-400">{t.blurb}</span>
                      </div>
                    ))}
                </div>
              </div>
            ) : (
              <>
                <ul className="divide-y divide-stone-200">
                  {installed.map(m => (
                    <li key={m.id} className="flex items-center gap-2 px-3 py-2">
                      <LuCpu className="h-3 w-3 text-stone-400" />
                      <span className="flex-1 truncate font-mono text-xs text-stone-800">
                        {m.id}
                      </span>
                      <span className="font-mono text-[10px] text-stone-400">
                        {formatBytes(m.sizeBytes)}
                      </span>
                      <button
                        className="rounded p-1 text-stone-400 hover:bg-coral-50 hover:text-coral-600"
                        aria-label="Remove model">
                        <LuTrash2 className="h-3 w-3" />
                      </button>
                    </li>
                  ))}
                </ul>
                <div className="space-y-2 border-t border-stone-200 px-3 py-2">
                  {/* There's no "pull arbitrary model by name" RPC today
                      — the existing local_ai_download_asset only pulls
                      whatever model is configured per capability slot.
                      So instead of a fake button, surface the real
                      workflow: browse Ollama's library, run `ollama pull`
                      from a terminal, and the installed-model list above
                      picks it up automatically on the next poll. */}
                  <button
                    onClick={() => {
                      void openUrl('https://ollama.com/library');
                    }}
                    className="inline-flex items-center gap-1 rounded-md border border-dashed border-stone-300 bg-white px-2 py-1 text-xs font-medium text-stone-600 hover:border-primary-400 hover:text-primary-700">
                    <LuDownload className="h-3 w-3" />
                    Browse Ollama library
                  </button>
                  <div className="text-[10px] text-stone-500">
                    To add a model, run{' '}
                    <span className="rounded bg-stone-100 px-1 py-0.5 font-mono text-[10px] text-stone-700">
                      ollama pull &lt;model&gt;
                    </span>{' '}
                    in your terminal. Installed models appear here within ~5s.
                  </div>
                </div>
              </>
            )}
          </div>

          {/* Daemon-conflict callout — surfaces the "external Ollama with
              broken runner" state in plain English so the user knows the
              recovery (kill external, retry) without having to read logs. */}
          {isDaemonConflict && ollama.state !== 'disabled' && (
            <div className="rounded-lg border border-amber-200 bg-amber-50 p-3 text-xs text-amber-900 space-y-2">
              <div className="font-medium">Conflicting Ollama daemon detected</div>
              <div>
                Another Ollama process is bound to <span className="font-mono">:11434</span> but
                OpenHuman didn&apos;t start it, so it can&apos;t safely restart it on your behalf.
                To recover:
              </div>
              <ol className="list-decimal pl-5 space-y-0.5">
                <li>
                  Stop the running Ollama (Windows Task Manager → end{' '}
                  <span className="font-mono">ollama.exe</span> /{' '}
                  <span className="font-mono">ollama app.exe</span>, or{' '}
                  <span className="font-mono">taskkill /F /IM ollama.exe</span>).
                </li>
                <li>
                  Click <span className="font-medium">Retry</span> above — OpenHuman will spawn its
                  own managed daemon.
                </li>
                <li>
                  Or, if you want to keep your install, set its binary path below — OpenHuman will
                  use yours.
                </li>
              </ol>
              {daemonWarning && (
                <div className="rounded border border-amber-200 bg-white/60 px-2 py-1 font-mono text-[10px] text-amber-800">
                  {daemonWarning}
                </div>
              )}
            </div>
          )}

          {/* Advanced: show the resolved binary path + let the user
              override it. The Rust resolver already supports a chain
              (user path → OLLAMA_BIN env → workspace bin → system PATH →
              auto-install); this surfaces it and provides one-field
              override. */}
          <details
            open={advancedOpen}
            onToggle={e => setAdvancedOpen((e.target as HTMLDetailsElement).open)}
            className="rounded-lg border border-stone-200 bg-white">
            <summary className="cursor-pointer select-none px-3 py-2 text-xs font-semibold uppercase tracking-wide text-stone-500 hover:text-stone-700">
              Advanced
            </summary>
            <div className="space-y-3 border-t border-stone-200 px-3 py-3">
              <div>
                <div className="text-[10px] font-semibold uppercase tracking-wide text-stone-500">
                  Resolved Ollama binary
                </div>
                <div className="mt-0.5 truncate font-mono text-[11px] text-stone-700">
                  {resolvedBinaryPath ||
                    '— not detected; OpenHuman will auto-install on next start'}
                </div>
              </div>
              <div>
                <label className="text-[10px] font-semibold uppercase tracking-wide text-stone-500">
                  Custom Ollama path
                </label>
                <div className="mt-1 flex gap-2">
                  <input
                    type="text"
                    value={customPathInput}
                    onChange={e => setCustomPathInput(e.target.value)}
                    placeholder="e.g. C:\Program Files\Ollama\ollama.exe"
                    className="min-w-0 flex-1 rounded-md border border-stone-300 bg-white px-2 py-1 font-mono text-[11px] text-stone-800 placeholder:text-stone-400 focus:border-primary-400 focus:outline-none focus:ring-1 focus:ring-primary-200"
                  />
                  <button
                    onClick={async () => {
                      setBusyAction('set-ollama-path');
                      try {
                        await localProvider.setBinaryPath(customPathInput.trim());
                      } catch (err) {
                        const msg = err instanceof Error ? err.message : String(err);
                        // eslint-disable-next-line no-console
                        console.warn('[ai-settings] set Ollama path failed', msg);
                      } finally {
                        setBusyAction(null);
                        await ollama.refresh();
                      }
                    }}
                    disabled={busyAction === 'set-ollama-path'}
                    className="rounded-md border border-stone-200 bg-white px-2.5 py-1 text-xs font-medium text-stone-700 hover:bg-stone-50 disabled:opacity-50">
                    Save
                  </button>
                  {customPathInput && (
                    <button
                      onClick={async () => {
                        setCustomPathInput('');
                        setBusyAction('set-ollama-path');
                        try {
                          await localProvider.setBinaryPath('');
                        } finally {
                          setBusyAction(null);
                          await ollama.refresh();
                        }
                      }}
                      disabled={busyAction === 'set-ollama-path'}
                      className="rounded-md border border-stone-200 bg-white px-2.5 py-1 text-xs font-medium text-stone-700 hover:bg-stone-50 disabled:opacity-50">
                      Clear
                    </button>
                  )}
                </div>
                <div className="mt-1 text-[10px] text-stone-500">
                  Empty = auto-detect (workspace install →{' '}
                  <span className="font-mono">OLLAMA_BIN</span> env → system{' '}
                  <span className="font-mono">PATH</span> → managed install).
                </div>
              </div>
            </div>
          </details>
        </section>

        {/* ─── Workload routing ────────────────────────────────────────── */}
        <section className="space-y-3">
          <div className="flex items-center justify-between gap-2">
            <SectionLabel>Workload routing</SectionLabel>
            <div className="inline-flex items-center rounded-full bg-stone-100 p-0.5">
              <button
                onClick={() => applyPreset('cloud')}
                className="rounded-full px-2 py-0.5 text-[10px] font-medium text-stone-600 hover:text-stone-900">
                Cloud
              </button>
              <button
                onClick={() => applyPreset('local')}
                className="rounded-full px-2 py-0.5 text-[10px] font-medium text-stone-600 hover:text-stone-900">
                Local
              </button>
              <button
                onClick={() => applyPreset('mixed')}
                className="rounded-full bg-white px-2 py-0.5 text-[10px] font-medium text-stone-900 shadow-subtle ring-1 ring-stone-200">
                Mixed
              </button>
            </div>
          </div>

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
                  />
                ))}
              </div>
            </div>
          </div>

          {primary && (
            <div className="text-[11px] text-stone-500">
              Primary resolves to{' '}
              <span className="font-mono text-stone-700">
                {primary.type === 'openhuman'
                  ? 'openhuman'
                  : `${PROVIDER_META[primary.type].label.toLowerCase()} · ${primary.defaultModel}`}
              </span>
            </div>
          )}
        </section>
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
                  // eslint-disable-next-line no-console
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
              // eslint-disable-next-line no-console
              console.warn('[ai-settings] clearCloudProviderKey failed', msg);
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
