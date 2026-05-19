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

import { listConnections as listComposioConnections } from '../../../lib/composio/composioApi';
import type { ComposioConnection } from '../../../lib/composio/types';
import { useT } from '../../../lib/i18n/I18nContext';
import {
  type AISettings as ApiAISettings,
  type ProviderRef as ApiProviderRef,
  clearCloudProviderKey,
  type CloudProviderView,
  flushCloudProviders,
  listProviderModels,
  loadAISettings,
  loadLocalProviderSnapshot,
  type LocalProviderSnapshot,
  type ModelInfo,
  saveAISettings,
  setCloudProviderKey,
} from '../../../services/api/aiSettingsApi';
import {
  creditsApi,
  type CreditTransaction,
  type TeamUsage,
} from '../../../services/api/creditsApi';
import {
  type AuthStyle,
  openhumanUpdateLocalAiSettings,
} from '../../../utils/tauriCommands/config';
import {
  type HeartbeatPlannerSummary,
  type HeartbeatSettings,
  type HeartbeatSettingsPatch,
  openhumanHeartbeatSettingsGet,
  openhumanHeartbeatSettingsSet,
  openhumanHeartbeatTickNow,
} from '../../../utils/tauriCommands/heartbeat';
import { ConfirmationModal } from '../../intelligence/ConfirmationModal';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import { useReembedBackfillModal } from './useReembedBackfillModal';

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
  | 'chat'
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
  | { kind: 'cloud'; providerSlug: string; model: string; temperature?: number | null }
  | { kind: 'local'; model: string; temperature?: number | null };

type Workload = { id: WorkloadId; group: WorkloadGroup; label: string; description: string };

type RoutingMap = Record<WorkloadId, ProviderRef>;

// ─────────────────────────────────────────────────────────────────────────────
// Static catalog
// ─────────────────────────────────────────────────────────────────────────────

// Slug-keyed display metadata for built-in provider slugs. Used only for
// chip rendering (label, tone). Custom providers use `provider.label` directly.
const BUILTIN_PROVIDER_META: Record<string, { tone: string; label: string }> = {
  openhuman: {
    label: 'OpenHuman',
    tone: 'bg-primary-50 dark:bg-primary-500/10 ring-primary-200 text-primary-900 dark:text-primary-100',
  },
  openai: {
    label: 'OpenAI',
    tone: 'bg-emerald-50 dark:bg-emerald-500/10 ring-emerald-200 text-emerald-900 dark:text-emerald-100',
  },
  anthropic: {
    label: 'Anthropic',
    tone: 'bg-orange-50 dark:bg-orange-500/10 ring-orange-200 text-orange-900 dark:text-orange-100',
  },
  openrouter: {
    label: 'OpenRouter',
    tone: 'bg-slate-100 dark:bg-slate-500/15 ring-slate-300 text-slate-900 dark:text-slate-100',
  },
  custom: {
    label: 'Custom',
    tone: 'bg-stone-100 dark:bg-neutral-800 ring-stone-300 text-stone-900 dark:text-neutral-100',
  },
};

const WORKLOADS: Workload[] = [
  { id: 'chat', group: 'chat', label: 'Chat', description: 'Direct conversational back-and-forth' },
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
  chat: { kind: 'openhuman' },
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
    chat: liftRef(api.routing.chat),
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
      chat: panel.routing.chat,
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
    // eslint-disable-next-line react-hooks/set-state-in-effect
    void reload();
  }, [reload]);

  // Eagerly persist user-configured cloud providers whenever they diverge from
  // the saved snapshot so listProviderModels can resolve by slug immediately
  // after a provider is added, before the global Save.
  //
  // Reserved slugs ("openhuman", "cloud", "pid") are built-ins that Rust
  // rejects as custom providers — filter them out before flushing. `ollama`
  // and `lmstudio` are NOT filtered: the AI panel needs an `ollama` entry on
  // disk for the model dropdown probe (`list_configured_models` looks up by
  // slug). Chat routing is unaffected because the factory's `ollama:<model>`
  // prefix branch fires before the `<slug>:<model>` cloud-provider lookup.
  useEffect(() => {
    if (loading) return;
    const userProviders = draft.cloudProviders.filter(
      p => !['', 'cloud', 'openhuman', 'pid'].includes(p.slug)
    );
    const savedUserProviders = saved.cloudProviders.filter(
      p => !['', 'cloud', 'openhuman', 'pid'].includes(p.slug)
    );
    if (JSON.stringify(userProviders) === JSON.stringify(savedUserProviders)) return;
    const wire = userProviders.map(p => ({
      id: p.id,
      slug: p.slug,
      label: p.label,
      endpoint: p.endpoint,
      auth_style: p.authStyle,
    }));
    flushCloudProviders(wire).catch(err =>
      console.warn('[ai-settings] eager cloud_providers flush failed:', err)
    );
  }, [draft.cloudProviders, loading, saved.cloudProviders]);

  const isDirty = JSON.stringify(saved) !== JSON.stringify(draft);

  const persist = useCallback(
    async (nextDraft: AISettings) => {
      const prevApi = toApiSettings(saved);
      const nextApi = toApiSettings(nextDraft);
      await saveAISettings(prevApi, nextApi);
      setSaved(nextDraft);
      setDraft(nextDraft);
      setError('');
    },
    [saved]
  );

  // Returns true only when persistence actually succeeded, so callers
  // (e.g. the #1574 re-embed-status check) don't act on a failed save.
  const save = useCallback(async (): Promise<boolean> => {
    try {
      // Defensive verification at global-Save time. Each provider that is new
      // or whose endpoint changed since the last saved snapshot is re-probed
      // through `openhuman.inference_list_models`. The chip / editor dialogs
      // already probe at add-time; this is a belt-and-suspenders check that
      // catches stale entries (endpoint flipped externally, daemon went
      // unreachable between add-time and save-time, etc.) before they reach
      // the saved config and start routing chat traffic to a dead host.
      //
      // OpenHuman is exempt (session JWT, no /models endpoint to hit).
      const savedById = new Map(saved.cloudProviders.map(p => [p.id, p]));
      const toProbe = draft.cloudProviders.filter(p => {
        if (p.slug === 'openhuman') return false;
        const prior = savedById.get(p.id);
        return !prior || prior.endpoint !== p.endpoint;
      });
      for (const p of toProbe) {
        try {
          await listProviderModels(p.slug);
        } catch (probeErr) {
          const msg = probeErr instanceof Error ? probeErr.message : String(probeErr);
          setError(`Could not reach ${p.label}: ${msg}. Settings were not saved.`);
          return false;
        }
      }

      await persist(draft);
      return true;
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to save AI settings';
      setError(message);
      return false;
    }
  }, [saved, draft, persist]);

  const discard = useCallback(() => setDraft(saved), [saved]);

  return { saved, draft, setDraft, isDirty, save, persist, discard, loading, error, reload };
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
    // eslint-disable-next-line react-hooks/set-state-in-effect
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
  lmstudio: 'bg-cyan-50 dark:bg-cyan-500/10 ring-cyan-200 text-cyan-900 dark:text-cyan-100',
  ollama: 'bg-violet-50 dark:bg-violet-500/10 ring-violet-200 text-violet-900 dark:text-violet-100',
};

const LOCAL_CHIP_LABEL: Record<LocalChipSlug, string> = { lmstudio: 'LM Studio', ollama: 'Ollama' };

function slugTone(slug: string): string {
  return (
    BUILTIN_PROVIDER_META[slug]?.tone ??
    'bg-stone-100 dark:bg-neutral-800 ring-stone-300 text-stone-900 dark:text-neutral-100'
  );
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
      className={`inline-flex items-center gap-2 rounded-full px-2.5 py-1 text-xs font-medium ring-1 transition-colors dark:ring-neutral-700 ${tone}`}>
      <span>{label}</span>
      <button
        type="button"
        role="switch"
        aria-checked={enabled}
        aria-label={`${enabled ? 'Disconnect' : 'Connect'} ${label}`}
        disabled={busy}
        onClick={onToggle}
        className={`relative inline-flex h-4 w-7 shrink-0 items-center rounded-full transition-colors disabled:cursor-wait disabled:opacity-60 ${enabled ? 'bg-primary-500' : 'bg-stone-300 dark:bg-neutral-700'}`}>
        <span
          aria-hidden
          className={`inline-block h-3 w-3 transform rounded-full bg-white dark:bg-neutral-900 shadow transition-transform ${enabled ? 'translate-x-3.5' : 'translate-x-0.5'}`}
        />
      </button>
    </div>
  );
};

// Connect-provider dialog — shown when the user flips a provider toggle ON.
//
// Two modes:
//   - apiKey: cloud providers (OpenAI, Anthropic, …). Collects a secret.
//   - endpoint: local runtimes (Ollama, LM Studio). Collects an HTTP URL
//     (and optionally an API key for OpenAI-compatible self-hosted setups).
//
// The parent decides how to persist: cloud → auth-profiles, local → both
// the cloud_providers entry's `endpoint` (so /models discovery works) and
// `local_ai.base_url` (so the Rust factory's Ollama branch routes to it).
const ProviderKeyDialog = ({
  slug,
  label,
  isLocalRuntime,
  onCancel,
  onSubmit,
}: {
  slug: string;
  label: string;
  /** When true, render an "Endpoint URL" field instead of API key. */
  isLocalRuntime: boolean;
  onCancel: () => void;
  /** Returns the entered value. For local runtimes this is the endpoint URL;
   *  for cloud providers it's the API key. */
  onSubmit: (value: string) => Promise<void> | void;
}) => {
  const { t } = useT();
  const [value, setValue] = useState<string>(isLocalRuntime ? defaultEndpointFor(slug) : '');
  const [phase, setPhase] = useState<'idle' | 'saving'>('idle');
  const [error, setError] = useState<string | null>(null);
  const busy = phase !== 'idle';

  const placeholder = isLocalRuntime
    ? defaultEndpointFor(slug) || 'http://localhost:11434/v1'
    : slug === 'openai'
      ? 'sk-...'
      : slug === 'anthropic'
        ? 'sk-ant-...'
        : slug === 'openrouter'
          ? 'sk-or-...'
          : 'your-api-key';

  const fieldLabel = isLocalRuntime ? 'Endpoint URL' : t('settings.ai.apiKeyFieldLabel');
  const helper = isLocalRuntime
    ? `Where ${label} is reachable. Default is localhost; point this at a remote host (e.g. http://10.0.0.4:11434/v1) to use a shared instance.`
    : t('settings.ai.apiKeyStoredEncrypted');

  const handleSave = async () => {
    const trimmed = value.trim();
    if (!trimmed) {
      setError(isLocalRuntime ? 'Endpoint URL is required' : t('settings.ai.apiKeyRequired'));
      return;
    }
    if (isLocalRuntime && !/^https?:\/\//i.test(trimmed)) {
      setError('Endpoint must start with http:// or https://');
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
      <div className="w-full max-w-md rounded-2xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 p-6 shadow-soft">
        <div className="mb-4">
          <h3 className="text-base font-semibold text-stone-900 dark:text-neutral-100">{`${t('settings.ai.connectProvider')} ${label}`}</h3>
          <p className="mt-0.5 text-xs text-stone-500 dark:text-neutral-400">{helper}</p>
        </div>

        <div className="flex flex-col gap-1.5">
          <label
            htmlFor="provider-key-input"
            className="text-xs font-medium text-stone-700 dark:text-neutral-200">
            {fieldLabel}
          </label>
          <input
            id="provider-key-input"
            type={isLocalRuntime ? 'url' : 'text'}
            autoComplete="off"
            autoCorrect="off"
            autoCapitalize="off"
            spellCheck={false}
            data-form-type="other"
            data-lpignore="true"
            data-1p-ignore="true"
            value={value}
            placeholder={placeholder}
            disabled={busy}
            onChange={e => {
              setValue(e.target.value);
              setError(null);
            }}
            className={`rounded-lg border border-stone-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 placeholder-stone-400 dark:placeholder-neutral-500 focus:border-primary-500 focus:outline-none focus:ring-1 focus:ring-primary-500 disabled:opacity-60 ${isLocalRuntime ? 'font-mono' : ''}`}
          />
          {error ? (
            <p className="text-xs font-medium text-red-600 dark:text-red-300">{error}</p>
          ) : null}
        </div>

        <div className="mt-6 flex justify-end gap-2">
          <button
            type="button"
            onClick={onCancel}
            disabled={busy}
            className="rounded-lg border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-4 py-2 text-sm font-medium text-stone-700 dark:text-neutral-200 hover:bg-stone-50 dark:hover:bg-neutral-800/60 dark:bg-neutral-800/60 dark:hover:bg-neutral-800/60 disabled:opacity-50">
            {t('common.cancel')}
          </button>
          <button
            type="button"
            onClick={() => void handleSave()}
            disabled={busy}
            className="rounded-lg bg-primary-500 px-4 py-2 text-sm font-medium text-white hover:bg-primary-600 disabled:cursor-not-allowed disabled:opacity-50">
            {phase === 'saving' ? t('settings.ai.saving') : t('common.save')}
          </button>
        </div>
      </div>
    </div>
  );
};

// ─────────────────────────────────────────────────────────────────────────────
// Background loop controls + usage diagnostics
// ─────────────────────────────────────────────────────────────────────────────

const USD = new Intl.NumberFormat('en-US', {
  style: 'currency',
  currency: 'USD',
  minimumFractionDigits: 4,
  maximumFractionDigits: 6,
});

const WEEK_MINUTES = 7 * 24 * 60;
const COMPOSIO_PERIODIC_TICK_MINUTES = 20;
const LEARNING_REBUILD_MINUTES = 30;
const MEMORY_WORKERS = 4;
const MEMORY_POLL_SECONDS = 5;

const formatUsd = (value: number): string => USD.format(Number.isFinite(value) ? value : 0);

const spendAmount = (tx: CreditTransaction): number => {
  const amount = Number(tx.amountUsd);
  return Number.isFinite(amount) ? Math.abs(amount) : 0;
};

const formatCount = (value: number): string =>
  new Intl.NumberFormat('en-US', { maximumFractionDigits: 0 }).format(
    Number.isFinite(value) ? value : 0
  );

const formatDateTime = (value: string | null | undefined): string => {
  if (!value) return 'n/a';
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return 'n/a';
  return date.toLocaleString([], {
    month: 'short',
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  });
};

const activeConnection = (connection: ComposioConnection): boolean => {
  const status = connection.status.toUpperCase();
  return status === 'ACTIVE' || status === 'CONNECTED';
};

const normalizedToolkit = (connection: ComposioConnection): string =>
  connection.toolkit.toLowerCase().replace(/[^a-z0-9]/g, '');

const isCalendarConnection = (connection: ComposioConnection): boolean => {
  const toolkit = normalizedToolkit(connection);
  return toolkit === 'googlecalendar' || toolkit === 'calendar';
};

function summarizeSpendByAction(
  transactions: CreditTransaction[]
): Array<[string, number, number]> {
  const byAction = new Map<string, { count: number; total: number }>();
  for (const tx of transactions) {
    if (tx.type !== 'SPEND') continue;
    const key = tx.action || 'SPEND';
    const prev = byAction.get(key) ?? { count: 0, total: 0 };
    prev.count += 1;
    prev.total += spendAmount(tx);
    byAction.set(key, prev);
  }
  return Array.from(byAction.entries())
    .map(([action, value]) => [action, value.count, value.total] as [string, number, number])
    .sort((a, b) => b[2] - a[2])
    .slice(0, 4);
}

function summarizeSpendByHour(transactions: CreditTransaction[]): Array<[string, number]> {
  const byHour = new Map<string, number>();
  for (const tx of transactions) {
    if (tx.type !== 'SPEND') continue;
    const date = new Date(tx.createdAt);
    if (Number.isNaN(date.getTime())) continue;
    date.setMinutes(0, 0, 0);
    const key = date.toLocaleString([], { month: 'short', day: 'numeric', hour: 'numeric' });
    byHour.set(key, (byHour.get(key) ?? 0) + spendAmount(tx));
  }
  return Array.from(byHour.entries())
    .sort((a, b) => b[1] - a[1])
    .slice(0, 4);
}

function summarizeSpendSample(transactions: CreditTransaction[]) {
  const rows = transactions
    .filter(tx => tx.type === 'SPEND')
    .sort((a, b) => new Date(b.createdAt).getTime() - new Date(a.createdAt).getTime());
  const total = rows.reduce((sum, tx) => sum + spendAmount(tx), 0);
  const avgRowUsd = rows.length > 0 ? total / rows.length : 0;
  const times = rows
    .map(tx => new Date(tx.createdAt).getTime())
    .filter(time => !Number.isNaN(time))
    .sort((a, b) => a - b);
  const sampleHours =
    times.length >= 2 ? Math.max((times[times.length - 1] - times[0]) / 3_600_000, 1 / 60) : 0;
  const spendPerHour = sampleHours > 0 ? total / sampleHours : 0;
  const rowsPerHour = sampleHours > 0 ? rows.length / sampleHours : 0;
  return { rows, total, avgRowUsd, sampleHours, spendPerHour, rowsPerHour };
}

function describeProvider(ref: ProviderRef, providers: CloudProvider[]): string {
  if (ref.kind === 'openhuman') return 'OpenHuman';
  if (ref.kind === 'local') return `Local ${ref.model}`;
  const provider = providers.find(p => p.slug === ref.providerSlug);
  return `${provider?.label ?? ref.providerSlug} ${ref.model || 'custom model'}`;
}

const LoopToggle = ({
  label,
  description,
  checked,
  busy,
  onToggle,
}: {
  label: string;
  description: string;
  checked: boolean;
  busy: boolean;
  onToggle: () => void;
}) => (
  <div className="flex items-center justify-between gap-3 rounded-lg border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-3 py-2">
    <div className="min-w-0">
      <div className="text-sm font-medium text-stone-900 dark:text-neutral-100">{label}</div>
      <div className="text-xs text-stone-500 dark:text-neutral-400">{description}</div>
    </div>
    <button
      type="button"
      role="switch"
      aria-label={label}
      aria-checked={checked}
      disabled={busy}
      onClick={onToggle}
      className={`relative inline-flex h-5 w-9 shrink-0 items-center rounded-full transition-colors disabled:cursor-wait disabled:opacity-60 ${checked ? 'bg-primary-500' : 'bg-stone-300 dark:bg-neutral-700'}`}>
      <span
        aria-hidden
        className={`inline-block h-4 w-4 transform rounded-full bg-white dark:bg-neutral-900 shadow transition-transform ${checked ? 'translate-x-4' : 'translate-x-0.5'}`}
      />
    </button>
  </div>
);

const MetricTile = ({
  label,
  value,
  detail,
}: {
  label: string;
  value: string;
  detail?: string;
}) => (
  <div className="rounded-md bg-stone-50 dark:bg-neutral-800/60 px-3 py-2">
    <div className="text-[10px] font-semibold uppercase tracking-wide text-stone-400 dark:text-neutral-500">
      {label}
    </div>
    <div className="mt-1 text-sm font-semibold text-stone-900 dark:text-neutral-100">{value}</div>
    {detail ? (
      <div className="mt-0.5 text-[11px] text-stone-500 dark:text-neutral-400">{detail}</div>
    ) : null}
  </div>
);

const FormulaRow = ({ label, value, detail }: { label: string; value: string; detail: string }) => (
  <div className="rounded-md border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-3 py-2">
    <div className="flex items-center justify-between gap-3">
      <span className="text-xs font-medium text-stone-800 dark:text-neutral-100">{label}</span>
      <span className="font-mono text-xs text-stone-600 dark:text-neutral-300">{value}</span>
    </div>
    <div className="mt-1 text-[11px] text-stone-500 dark:text-neutral-400">{detail}</div>
  </div>
);

const BackgroundLoopControls = ({
  routing,
  cloudProviders,
}: {
  routing: RoutingMap;
  cloudProviders: CloudProvider[];
}) => {
  const [settings, setSettings] = useState<HeartbeatSettings | null>(null);
  const [usage, setUsage] = useState<TeamUsage | null>(null);
  const [transactions, setTransactions] = useState<CreditTransaction[]>([]);
  const [connections, setConnections] = useState<ComposioConnection[]>([]);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState<string | null>(null);
  const [runningTick, setRunningTick] = useState(false);
  const [plannerSummary, setPlannerSummary] = useState<HeartbeatPlannerSummary | null>(null);
  const [error, setError] = useState<string>('');
  const settingsRef = useRef<HeartbeatSettings | null>(null);
  const patchRequestIdRef = useRef(0);

  const commitSettings = useCallback((nextSettings: HeartbeatSettings | null) => {
    settingsRef.current = nextSettings;
    setSettings(nextSettings);
  }, []);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError('');
    const [heartbeatResult, usageResult, transactionsResult, connectionsResult] =
      await Promise.allSettled([
        openhumanHeartbeatSettingsGet(),
        creditsApi.getTeamUsage(),
        creditsApi.getTransactions(200, 0),
        listComposioConnections(),
      ]);

    if (heartbeatResult.status === 'fulfilled') {
      commitSettings(heartbeatResult.value.result.settings);
    } else {
      setError(
        heartbeatResult.reason instanceof Error ? heartbeatResult.reason.message : 'Load failed'
      );
    }

    if (usageResult.status === 'fulfilled') {
      setUsage(usageResult.value);
    }

    if (transactionsResult.status === 'fulfilled') {
      setTransactions(transactionsResult.value.transactions ?? []);
    }

    if (connectionsResult.status === 'fulfilled') {
      setConnections(connectionsResult.value.connections ?? []);
    }
    setLoading(false);
  }, [commitSettings]);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect
    void refresh();
  }, [refresh]);

  const applyHeartbeatPatch = useCallback(
    async (patch: HeartbeatSettingsPatch) => {
      const requestId = patchRequestIdRef.current + 1;
      patchRequestIdRef.current = requestId;
      const savingKey = Object.keys(patch).join(',');
      const previous = settingsRef.current;
      setError('');
      setSaving(savingKey);
      if (!previous) {
        // No baseline to patch against — abandon this request.
        if (patchRequestIdRef.current === requestId) {
          setSaving(null);
        }
        return;
      }
      commitSettings({ ...previous, ...patch });
      try {
        const response = await openhumanHeartbeatSettingsSet(patch);
        // Stale response — a newer patch superseded us; drop this result.
        if (patchRequestIdRef.current !== requestId) return;
        commitSettings(response.result.settings);
      } catch (err) {
        if (patchRequestIdRef.current !== requestId) return;
        commitSettings(previous);
        setError(err instanceof Error ? err.message : String(err));
      } finally {
        if (patchRequestIdRef.current === requestId) {
          setSaving(null);
        }
      }
    },
    [commitSettings]
  );

  const runPlannerNow = useCallback(async () => {
    setRunningTick(true);
    setError('');
    try {
      const response = await openhumanHeartbeatTickNow();
      setPlannerSummary(response.result.summary);
      await refresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setRunningTick(false);
    }
  }, [refresh]);

  const spendSample = summarizeSpendSample(transactions);
  const spendRows = spendSample.rows;
  const actionSummary = summarizeSpendByAction(transactions);
  const hourSummary = summarizeSpendByHour(transactions);
  const latestSpend = spendRows[0] ?? null;
  const heartbeatIntervalMinutes = settings ? Math.max(settings.interval_minutes, 5) : 5;
  const heartbeatTicksPerWeek = settings?.enabled
    ? Math.ceil(WEEK_MINUTES / heartbeatIntervalMinutes)
    : 0;
  const activeConnections = connections.filter(activeConnection);
  const activeCalendarConnections = activeConnections.filter(isCalendarConnection);
  const maxCalendarConnectionsPerTick = settings
    ? Math.max(settings.max_calendar_connections_per_tick ?? 2, 1)
    : 2;
  const calendarConnectionsPolled = settings?.notify_meetings
    ? Math.min(activeCalendarConnections.length, maxCalendarConnectionsPerTick)
    : 0;
  const calendarConnectionsSkipped = settings?.notify_meetings
    ? Math.max(activeCalendarConnections.length - calendarConnectionsPolled, 0)
    : 0;
  const calendarPlannerCallsPerTick = settings?.notify_meetings ? 1 + calendarConnectionsPolled : 0;
  const calendarPlannerCallsPerWeek = heartbeatTicksPerWeek * calendarPlannerCallsPerTick;
  const subconsciousModelCallsPerWeek =
    settings?.enabled && settings.inference_enabled ? heartbeatTicksPerWeek : 0;
  const composioPeriodicTicksPerWeek = Math.ceil(WEEK_MINUTES / COMPOSIO_PERIODIC_TICK_MINUTES);
  const learningTicksPerWeek = Math.ceil(WEEK_MINUTES / LEARNING_REBUILD_MINUTES);
  const memoryPollsPerWeek = Math.ceil((WEEK_MINUTES * 60 * MEMORY_WORKERS) / MEMORY_POLL_SECONDS);
  const composioConnectionScansPerWeek = composioPeriodicTicksPerWeek * activeConnections.length;
  const backgroundApiReadsPerWeek = calendarPlannerCallsPerWeek + composioConnectionScansPerWeek;
  const backgroundWakeupsPerWeek =
    heartbeatTicksPerWeek +
    composioPeriodicTicksPerWeek +
    learningTicksPerWeek +
    memoryPollsPerWeek;
  const scheduledCallsPerRemainingDollar =
    usage && usage.remainingUsd > 0 ? backgroundApiReadsPerWeek / usage.remainingUsd : null;
  const estimatedRowsLeft =
    usage && spendSample.avgRowUsd > 0
      ? Math.floor(usage.remainingUsd / spendSample.avgRowUsd)
      : null;
  const estimatedRowsPerBudget =
    usage && spendSample.avgRowUsd > 0
      ? Math.floor(usage.cycleBudgetUsd / spendSample.avgRowUsd)
      : null;
  const projectedHoursLeft =
    usage && spendSample.spendPerHour > 0 ? usage.remainingUsd / spendSample.spendPerHour : null;
  const projectionAnchorMs = latestSpend ? new Date(latestSpend.createdAt).getTime() : Number.NaN;
  const projectedExhaustAt =
    projectedHoursLeft !== null && Number.isFinite(projectionAnchorMs)
      ? new Date(projectionAnchorMs + projectedHoursLeft * 3_600_000).toLocaleString([], {
          month: 'short',
          day: 'numeric',
          hour: 'numeric',
          minute: '2-digit',
        })
      : 'n/a';

  const loops = [
    {
      name: 'Heartbeat planner',
      enabled: Boolean(settings?.enabled),
      cadence: `${settings?.interval_minutes ?? 5} min`,
      route: describeProvider(routing.heartbeat, cloudProviders),
      work: 'Runs proactive collectors: cron reminders, calendar meetings, relevant notifications.',
      risk: settings?.notify_meetings
        ? `${calendarPlannerCallsPerTick} Composio read call(s)/tick; ${calendarConnectionsSkipped} calendar link(s) over cap skipped.`
        : 'Calendar collector off; planner reads only local enabled categories.',
    },
    {
      name: 'Subconscious tick',
      enabled: Boolean(settings?.enabled && settings?.inference_enabled),
      cadence: `${settings?.interval_minutes ?? 5} min`,
      route: describeProvider(routing.subconscious, cloudProviders),
      work: 'Evaluates subconscious tasks/reflections through kind=subconscious_tick.',
      risk:
        subconsciousModelCallsPerWeek > 0
          ? `${formatCount(subconsciousModelCallsPerWeek)} model call(s)/week at current interval.`
          : 'Inference off; no scheduled subconscious model calls.',
    },
    {
      name: 'Memory tree workers',
      enabled: true,
      cadence: 'queue',
      route: describeProvider(routing.memory, cloudProviders),
      work: 'Extracts chunks, seals branches, runs daily digests, routes topics.',
      risk: `${MEMORY_WORKERS} workers poll every ${MEMORY_POLL_SECONDS}s; LLM calls only when queue has extract/seal/digest/topic jobs.`,
    },
    {
      name: 'Reflection rebuild',
      enabled: true,
      cadence: '30 min',
      route: describeProvider(routing.learning, cloudProviders),
      work: 'Refreshes reflection state after memory activity.',
      risk: `${formatCount(learningTicksPerWeek)} wakeups/week; LLM work only when rebuild needs reflection.`,
    },
    {
      name: 'Composio sync',
      enabled: true,
      cadence: '20 min',
      route: 'Integration APIs',
      work: 'Polls connected tools when provider sync is due.',
      risk: `${formatCount(composioPeriodicTicksPerWeek)} wakeups/week; scans ${activeConnections.length} active connection(s).`,
    },
  ];

  return (
    <div className="space-y-4">
      <div className="border-b border-stone-200 dark:border-neutral-800 pb-2">
        <h2 className="text-base font-semibold text-stone-900 dark:text-neutral-100">
          Background loops
        </h2>
        <p className="mt-0.5 text-xs text-stone-500 dark:text-neutral-400">
          See what runs without a chat message, pause heartbeat work, and inspect recent credit
          ledger rows.
        </p>
      </div>

      {error && (
        <div className="rounded-md border border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 px-3 py-2 text-xs text-coral-700 dark:text-coral-300">
          {error}
        </div>
      )}

      <section className="grid gap-3 lg:grid-cols-[minmax(0,1fr)_minmax(300px,0.8fr)]">
        <div className="space-y-3">
          <div className="rounded-lg border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 p-3">
            <div className="mb-3 flex items-center justify-between gap-3">
              <div>
                <div className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
                  Heartbeat controls
                </div>
                <div className="text-xs text-stone-500 dark:text-neutral-400">
                  Defaults off. Enabling starts the loop; disabling aborts the running task.
                </div>
              </div>
              <button
                type="button"
                onClick={() => void refresh()}
                disabled={loading}
                className="rounded-md border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-2 py-1 text-xs font-medium text-stone-700 dark:text-neutral-200 hover:bg-stone-50 dark:hover:bg-neutral-800/60 dark:bg-neutral-800/60 dark:hover:bg-neutral-800/60 disabled:opacity-50">
                Refresh
              </button>
            </div>

            {settings ? (
              <div className="space-y-2">
                <LoopToggle
                  label="Heartbeat loop"
                  description="Master scheduler for planner + optional subconscious inference."
                  checked={settings.enabled}
                  busy={saving === 'enabled'}
                  onToggle={() => void applyHeartbeatPatch({ enabled: !settings.enabled })}
                />
                <LoopToggle
                  label="Subconscious inference"
                  description="Runs model-backed task/reflection evaluation on heartbeat ticks."
                  checked={settings.inference_enabled}
                  busy={saving === 'inference_enabled'}
                  onToggle={() =>
                    void applyHeartbeatPatch({ inference_enabled: !settings.inference_enabled })
                  }
                />
                <LoopToggle
                  label="Calendar meeting checks"
                  description="Calls calendar event list for active Google Calendar connections."
                  checked={settings.notify_meetings}
                  busy={saving === 'notify_meetings'}
                  onToggle={() =>
                    void applyHeartbeatPatch({ notify_meetings: !settings.notify_meetings })
                  }
                />
                <div className="grid gap-2 rounded-lg border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-3 py-2 md:grid-cols-3">
                  <label className="space-y-1 text-xs font-medium text-stone-700 dark:text-neutral-200">
                    <span>Calendar cap</span>
                    <select
                      value={maxCalendarConnectionsPerTick}
                      disabled={saving === 'max_calendar_connections_per_tick'}
                      onChange={e =>
                        void applyHeartbeatPatch({
                          max_calendar_connections_per_tick: Number(e.target.value),
                        })
                      }
                      className="w-full rounded-md border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-2 py-1 text-xs text-stone-900 dark:text-neutral-100 focus:border-primary-500 focus:outline-none focus:ring-1 focus:ring-primary-500">
                      {[1, 2, 3, 5, 10].map(count => (
                        <option key={count} value={count}>
                          {count} conn/tick
                        </option>
                      ))}
                    </select>
                  </label>
                  <label className="space-y-1 text-xs font-medium text-stone-700 dark:text-neutral-200">
                    <span>Meeting lookahead</span>
                    <select
                      value={settings.meeting_lookahead_minutes}
                      disabled={saving === 'meeting_lookahead_minutes'}
                      onChange={e =>
                        void applyHeartbeatPatch({
                          meeting_lookahead_minutes: Number(e.target.value),
                        })
                      }
                      className="w-full rounded-md border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-2 py-1 text-xs text-stone-900 dark:text-neutral-100 focus:border-primary-500 focus:outline-none focus:ring-1 focus:ring-primary-500">
                      {[15, 30, 60, 120, 240].map(minutes => (
                        <option key={minutes} value={minutes}>
                          {minutes} min
                        </option>
                      ))}
                    </select>
                  </label>
                  <label className="space-y-1 text-xs font-medium text-stone-700 dark:text-neutral-200">
                    <span>Reminder lookahead</span>
                    <select
                      value={settings.reminder_lookahead_minutes}
                      disabled={saving === 'reminder_lookahead_minutes'}
                      onChange={e =>
                        void applyHeartbeatPatch({
                          reminder_lookahead_minutes: Number(e.target.value),
                        })
                      }
                      className="w-full rounded-md border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-2 py-1 text-xs text-stone-900 dark:text-neutral-100 focus:border-primary-500 focus:outline-none focus:ring-1 focus:ring-primary-500">
                      {[5, 15, 30, 60, 120].map(minutes => (
                        <option key={minutes} value={minutes}>
                          {minutes} min
                        </option>
                      ))}
                    </select>
                  </label>
                </div>
                <LoopToggle
                  label="Cron reminder checks"
                  description="Scans enabled cron jobs for reminder-like upcoming items."
                  checked={settings.notify_reminders}
                  busy={saving === 'notify_reminders'}
                  onToggle={() =>
                    void applyHeartbeatPatch({ notify_reminders: !settings.notify_reminders })
                  }
                />
                <LoopToggle
                  label="Relevant notification checks"
                  description="Promotes urgent local notifications into proactive alerts."
                  checked={settings.notify_relevant_events}
                  busy={saving === 'notify_relevant_events'}
                  onToggle={() =>
                    void applyHeartbeatPatch({
                      notify_relevant_events: !settings.notify_relevant_events,
                    })
                  }
                />
                <LoopToggle
                  label="External delivery"
                  description="Lets heartbeat alerts send proactive messages to external channels."
                  checked={settings.external_delivery_enabled}
                  busy={saving === 'external_delivery_enabled'}
                  onToggle={() =>
                    void applyHeartbeatPatch({
                      external_delivery_enabled: !settings.external_delivery_enabled,
                    })
                  }
                />

                <div className="flex flex-wrap items-center gap-2 rounded-lg border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-3 py-2">
                  <label
                    className="text-xs font-medium text-stone-700 dark:text-neutral-200"
                    htmlFor="heartbeat-interval">
                    Interval
                  </label>
                  <select
                    id="heartbeat-interval"
                    value={settings.interval_minutes}
                    disabled={saving === 'interval_minutes'}
                    onChange={e =>
                      void applyHeartbeatPatch({ interval_minutes: Number(e.target.value) })
                    }
                    className="rounded-md border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-2 py-1 text-xs text-stone-900 dark:text-neutral-100 focus:border-primary-500 focus:outline-none focus:ring-1 focus:ring-primary-500">
                    {[5, 10, 15, 30, 60].map(minutes => (
                      <option key={minutes} value={minutes}>
                        {minutes} min
                      </option>
                    ))}
                  </select>
                  <button
                    type="button"
                    onClick={() => void runPlannerNow()}
                    disabled={runningTick}
                    className="ml-auto rounded-md border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-2 py-1 text-xs font-medium text-stone-700 dark:text-neutral-200 hover:bg-stone-50 dark:hover:bg-neutral-800/60 dark:bg-neutral-800/60 dark:hover:bg-neutral-800/60 disabled:opacity-50">
                    {runningTick ? 'Running...' : 'Planner tick now'}
                  </button>
                </div>

                {plannerSummary && (
                  <div className="rounded-md border border-primary-100 bg-primary-50 dark:bg-primary-500/10 px-3 py-2 text-xs text-primary-900">
                    Planner: {plannerSummary.source_events} source events,{' '}
                    {plannerSummary.deliveries_sent} sent, {plannerSummary.deliveries_skipped_dedup}{' '}
                    deduped.
                  </div>
                )}
              </div>
            ) : (
              <div className="text-xs text-stone-500 dark:text-neutral-400">
                {loading ? 'Loading heartbeat controls...' : 'Heartbeat controls unavailable.'}
              </div>
            )}
          </div>

          <div className="overflow-hidden rounded-lg border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60">
            <div className="border-b border-stone-200 dark:border-neutral-800 px-3 py-2 text-xs font-semibold uppercase tracking-wide text-stone-400 dark:text-neutral-500">
              Loop map
            </div>
            <div className="divide-y divide-stone-200 dark:divide-neutral-800">
              {loops.map(loop => (
                <div key={loop.name} className="grid gap-2 px-3 py-3 md:grid-cols-[150px_1fr]">
                  <div>
                    <div className="text-sm font-medium text-stone-900 dark:text-neutral-100">
                      {loop.name}
                    </div>
                    <div className="mt-0.5 flex flex-wrap gap-1 text-[11px] text-stone-500 dark:text-neutral-400">
                      <span>{loop.enabled ? 'on' : 'off'}</span>
                      <span>{loop.cadence}</span>
                    </div>
                  </div>
                  <div className="text-xs text-stone-600 dark:text-neutral-300">
                    <div>{loop.work}</div>
                    <div className="mt-1 font-mono text-[11px] text-stone-500 dark:text-neutral-400">
                      route: {loop.route}
                    </div>
                    <div className="mt-1 text-stone-500 dark:text-neutral-400">{loop.risk}</div>
                  </div>
                </div>
              ))}
            </div>
          </div>
        </div>

        <div className="rounded-lg border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 p-3">
          <div className="flex items-center justify-between gap-3">
            <div>
              <div className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
                Recent usage ledger
              </div>
              <div className="text-xs text-stone-500 dark:text-neutral-400">
                Backend rows expose action/time today; source tags need backend support.
              </div>
            </div>
            <button
              type="button"
              onClick={() => void refresh()}
              disabled={loading}
              className="rounded-md border border-stone-200 dark:border-neutral-800 px-2 py-1 text-xs font-medium text-stone-700 dark:text-neutral-200 hover:bg-stone-50 dark:hover:bg-neutral-800/60 dark:bg-neutral-800/60 dark:hover:bg-neutral-800/60 disabled:opacity-50">
              Reload
            </button>
          </div>

          <div className="mt-3 grid grid-cols-2 gap-2 md:grid-cols-3">
            <MetricTile
              label="Week budget"
              value={usage ? formatUsd(usage.cycleBudgetUsd) : 'n/a'}
              detail={`resets ${formatDateTime(usage?.cycleEndsAt)}`}
            />
            <MetricTile
              label="Cycle remaining"
              value={usage ? formatUsd(usage.remainingUsd) : 'n/a'}
              detail={usage ? `${formatUsd(usage.cycleSpentUsd)} used` : undefined}
            />
            <MetricTile
              label="Cycle total spend"
              value={usage ? formatUsd(usage.insights.totals.totalUsd) : 'n/a'}
              detail={
                usage
                  ? `inference ${formatUsd(usage.insights.totals.inferenceUsd)} + integrations ${formatUsd(usage.insights.totals.integrationsUsd)}`
                  : undefined
              }
            />
            <MetricTile
              label="Avg spend row"
              value={spendSample.avgRowUsd > 0 ? formatUsd(spendSample.avgRowUsd) : 'n/a'}
              detail={`${spendRows.length} recent spend rows`}
            />
            <MetricTile
              label="Bg API reads"
              value={`${formatCount(backgroundApiReadsPerWeek)}/week`}
              detail={`${formatCount(calendarPlannerCallsPerWeek)} planner + ${formatCount(composioConnectionScansPerWeek)} sync`}
            />
            <MetricTile
              label="Bg wakeups"
              value={`${formatCount(backgroundWakeupsPerWeek)}/week`}
              detail={`${formatCount(memoryPollsPerWeek)} memory polls`}
            />
          </div>

          <div className="mt-3 rounded-lg border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 p-3">
            <div className="text-[10px] font-semibold uppercase tracking-wide text-stone-400 dark:text-neutral-500">
              Budget math
            </div>
            <div className="mt-2 grid gap-2">
              <FormulaRow
                label="Rows left"
                value={estimatedRowsLeft !== null ? formatCount(estimatedRowsLeft) : 'n/a'}
                detail={
                  estimatedRowsLeft !== null
                    ? `remaining / avg row = ${formatUsd(usage?.remainingUsd ?? 0)} / ${formatUsd(spendSample.avgRowUsd)}`
                    : 'Need recent spend rows to estimate.'
                }
              />
              <FormulaRow
                label="Rows per full week budget"
                value={
                  estimatedRowsPerBudget !== null ? formatCount(estimatedRowsPerBudget) : 'n/a'
                }
                detail={
                  estimatedRowsPerBudget !== null
                    ? `cycle budget / avg row = ${formatUsd(usage?.cycleBudgetUsd ?? 0)} / ${formatUsd(spendSample.avgRowUsd)}`
                    : 'Need recent spend rows to estimate.'
                }
              />
              <FormulaRow
                label="Sample burn rate"
                value={
                  spendSample.spendPerHour > 0 ? `${formatUsd(spendSample.spendPerHour)}/hr` : 'n/a'
                }
                detail={
                  spendSample.sampleHours > 0
                    ? `${formatCount(spendSample.rowsPerHour)} rows/hr across ${spendSample.sampleHours.toFixed(1)}h sample`
                    : 'Need timestamps from at least two spend rows.'
                }
              />
              <FormulaRow
                label="Projected empty"
                value={projectedExhaustAt}
                detail={
                  projectedHoursLeft !== null
                    ? `${projectedHoursLeft.toFixed(1)}h after latest spend at recent burn rate`
                    : 'No projection without recent hourly spend.'
                }
              />
              <FormulaRow
                label="API reads per $ remaining"
                value={
                  scheduledCallsPerRemainingDollar !== null
                    ? `${formatCount(scheduledCallsPerRemainingDollar)} reads/$`
                    : 'n/a'
                }
                detail={
                  usage
                    ? `background API reads/week / remaining = ${formatCount(backgroundApiReadsPerWeek)} / ${formatUsd(usage.remainingUsd)}`
                    : 'Need usage response to estimate.'
                }
              />
            </div>
          </div>

          <div className="mt-3 rounded-lg border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 p-3">
            <div className="text-[10px] font-semibold uppercase tracking-wide text-stone-400 dark:text-neutral-500">
              Loop call budget
            </div>
            <div className="mt-2 grid gap-2">
              <FormulaRow
                label="Heartbeat ticks"
                value={`${formatCount(heartbeatTicksPerWeek)}/week`}
                detail={`10080 min/week / ${heartbeatIntervalMinutes} min interval`}
              />
              <FormulaRow
                label="Calendar planner calls"
                value={`${formatCount(calendarPlannerCallsPerWeek)}/week`}
                detail={
                  settings?.notify_meetings
                    ? `ticks * (1 list_connections + ${calendarConnectionsPolled} GOOGLECALENDAR_EVENTS_LIST)`
                    : 'Meeting collector disabled.'
                }
              />
              <FormulaRow
                label="Calendar fanout cap"
                value={`${formatCount(calendarConnectionsPolled)}/${formatCount(activeCalendarConnections.length)} conn/tick`}
                detail={`max_calendar_connections_per_tick = ${maxCalendarConnectionsPerTick}; skipped now = ${calendarConnectionsSkipped}`}
              />
              <FormulaRow
                label="Subconscious model calls"
                value={`${formatCount(subconsciousModelCallsPerWeek)}/week`}
                detail={
                  settings?.enabled && settings.inference_enabled
                    ? 'one kind=subconscious_tick model call per heartbeat tick'
                    : 'Heartbeat inference disabled.'
                }
              />
              <FormulaRow
                label="Composio sync scans"
                value={`${formatCount(composioConnectionScansPerWeek)}/week`}
                detail={`${activeConnections.length} active integration connection(s) scanned every ${COMPOSIO_PERIODIC_TICK_MINUTES} min`}
              />
              <FormulaRow
                label="Total bg API read budget"
                value={`${formatCount(backgroundApiReadsPerWeek)}/week`}
                detail={`calendar planner reads + periodic integration scans; excludes user-initiated chat tools`}
              />
              <FormulaRow
                label="Memory worker polls"
                value={`${formatCount(memoryPollsPerWeek)}/week max`}
                detail={`${MEMORY_WORKERS} workers * ${MEMORY_POLL_SECONDS}s poll; LLM calls only for queued jobs`}
              />
            </div>
          </div>

          {latestSpend && (
            <div className="mt-3 rounded-md border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 px-3 py-2 text-xs text-stone-600 dark:text-neutral-300">
              Latest spend: {formatUsd(spendAmount(latestSpend))} at{' '}
              {new Date(latestSpend.createdAt).toLocaleString()} ({latestSpend.action})
            </div>
          )}

          <div className="mt-3 space-y-3">
            <div>
              <div className="text-[10px] font-semibold uppercase tracking-wide text-stone-400 dark:text-neutral-500">
                Top actions
              </div>
              <div className="mt-1 space-y-1">
                {actionSummary.length > 0 ? (
                  actionSummary.map(([action, count, total]) => (
                    <div
                      key={action}
                      className="flex items-center justify-between gap-2 text-xs text-stone-600 dark:text-neutral-300">
                      <span className="truncate font-mono">{action}</span>
                      <span className="shrink-0 text-stone-500 dark:text-neutral-400">
                        {count} / {formatUsd(total)}
                      </span>
                    </div>
                  ))
                ) : (
                  <div className="text-xs text-stone-500 dark:text-neutral-400">
                    No spend rows loaded.
                  </div>
                )}
              </div>
            </div>

            <div>
              <div className="text-[10px] font-semibold uppercase tracking-wide text-stone-400 dark:text-neutral-500">
                Top hours
              </div>
              <div className="mt-1 space-y-1">
                {hourSummary.length > 0 ? (
                  hourSummary.map(([hour, total]) => (
                    <div
                      key={hour}
                      className="flex items-center justify-between gap-2 text-xs text-stone-600 dark:text-neutral-300">
                      <span>{hour}</span>
                      <span className="font-mono text-stone-500 dark:text-neutral-400">
                        {formatUsd(total)}
                      </span>
                    </div>
                  ))
                ) : (
                  <div className="text-xs text-stone-500 dark:text-neutral-400">
                    No hourly spend yet.
                  </div>
                )}
              </div>
            </div>
          </div>
        </div>
      </section>
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
  const { t } = useT();
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
  const activeSegment =
    'bg-white dark:bg-neutral-900 text-stone-900 dark:text-neutral-100 shadow-subtle ring-1 ring-stone-200 dark:ring-neutral-600';
  const inactiveSegment =
    'text-stone-500 dark:text-neutral-400 hover:text-stone-800 dark:text-neutral-100 dark:hover:text-neutral-200';

  return (
    <div className="flex items-center justify-between gap-3 py-3">
      <div className="min-w-0 flex-1">
        <div className="text-sm font-medium text-stone-900 dark:text-neutral-100">
          {workload.label}
        </div>
        <div className="truncate text-xs text-stone-500 dark:text-neutral-400">
          {workload.description}
        </div>
        <div className="mt-0.5 font-mono text-[11px] text-stone-400 dark:text-neutral-500 truncate">
          ↳ {resolved}
        </div>
      </div>
      <div className="inline-flex shrink-0 items-center rounded-lg bg-stone-100 dark:bg-neutral-800 p-0.5">
        <button
          type="button"
          onClick={() => onChange({ kind: 'openhuman' })}
          className={`${segmentBase} ${isDefault ? activeSegment : inactiveSegment}`}>
          {t('settings.ai.routingDefault')}
        </button>
        <button
          type="button"
          onClick={onCustomClick}
          className={`${segmentBase} ${!isDefault ? activeSegment : inactiveSegment}`}>
          {t('settings.ai.routingCustom')}
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

function humanizeModelId(id: string): string {
  return id.replace(/[-_]/g, ' ').replace(/\b\w/g, c => c.toUpperCase());
}

const CustomRoutingDialog = ({
  workload,
  initial,
  cloudProviders,
  localModels,
  ollamaRunning,
  onClose,
  onSubmit,
}: CustomRoutingDialogProps) => {
  const { t } = useT();
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
  const [cloudModels, setCloudModels] = useState<ModelInfo[]>([]);
  const [cloudModelsLoading, setCloudModelsLoading] = useState(false);
  const [cloudModelsError, setCloudModelsError] = useState<string | null>(null);
  const [modelsKey, setModelsKey] = useState(0);
  // Optional temperature override for this workload. `null` = use provider/global default;
  // a finite number means "send `temperature: X` upstream for this workload only".
  const [temperature, setTemperature] = useState<number | null>(
    initial.kind === 'cloud' || initial.kind === 'local' ? (initial.temperature ?? null) : null
  );

  const selectedCloud =
    source?.kind === 'cloud' ? customCloud.find(c => c.slug === source.providerSlug) : undefined;

  // Fetch available models whenever the selected cloud provider changes.
  const selectedSlug = source?.kind === 'cloud' ? source.providerSlug : null;
  useEffect(() => {
    if (!selectedSlug) {
      // eslint-disable-next-line react-hooks/set-state-in-effect
      setCloudModels([]);
      setCloudModelsError(null);
      return;
    }
    const provider = customCloud.find(c => c.slug === selectedSlug);
    if (!provider) {
      setCloudModels([]);
      setCloudModelsError(null);
      return;
    }
    let active = true;
    setCloudModelsLoading(true);
    setCloudModels([]);
    setCloudModelsError(null);
    console.debug('[ai-settings] fetching models for provider', provider.slug);
    listProviderModels(provider.slug)
      .then(ms => {
        if (!active) return;
        console.debug('[ai-settings] fetched', ms.length, 'models for', provider.slug);
        setCloudModels(ms);
        setCloudModelsLoading(false);
      })
      .catch((err: unknown) => {
        if (!active) return;
        const msg = err instanceof Error ? err.message : String(err);
        console.error('[ai-settings] listProviderModels failed for', provider.slug, ':', msg);
        setCloudModelsError(msg);
        setCloudModelsLoading(false);
      });
    return () => {
      active = false;
    };
    // customCloud is stable for the dialog's lifetime (prop doesn't change mid-open)
    // modelsKey is the manual retry trigger
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedSlug, modelsKey]);

  const canSave = source !== null && model.trim().length > 0;

  const handleSave = () => {
    if (!source || !canSave) return;
    const temp = temperature == null || !Number.isFinite(temperature) ? null : temperature;
    if (source.kind === 'cloud') {
      onSubmit({
        kind: 'cloud',
        providerSlug: source.providerSlug,
        model: model.trim(),
        temperature: temp,
      });
    } else {
      onSubmit({ kind: 'local', model: model.trim(), temperature: temp });
    }
  };

  const noProviders = customCloud.length === 0 && !localAvailable;

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label={`Custom routing for ${workload.label}`}
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/30 p-4">
      <div className="w-full max-w-md rounded-2xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 p-6 shadow-soft">
        <div className="flex items-start justify-between gap-3 mb-4">
          <div>
            <h3 className="text-base font-semibold text-stone-900 dark:text-neutral-100">
              {t('settings.ai.customRouting')}
            </h3>
            <p className="mt-0.5 text-xs text-stone-500 dark:text-neutral-400">{workload.label}</p>
          </div>
          <button
            type="button"
            onClick={onClose}
            className="rounded-md p-1 text-stone-400 dark:text-neutral-500 hover:bg-stone-100 dark:hover:bg-neutral-800 dark:bg-neutral-800 dark:hover:bg-neutral-800/60 hover:text-stone-700 dark:hover:text-neutral-200 dark:text-neutral-200 dark:hover:text-neutral-200">
            <span className="sr-only">{t('common.close')}</span>
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
          <div className="rounded-lg border border-amber-200 dark:border-amber-500/30 bg-amber-50 dark:bg-amber-500/10 p-3 text-xs text-amber-800 dark:text-amber-200">
            {t('settings.ai.noCustomProviders')}
          </div>
        ) : (
          <div className="flex flex-col gap-4">
            <div className="flex flex-col gap-1.5">
              <label className="text-xs font-medium text-stone-700 dark:text-neutral-200">
                {t('settings.ai.providerLabel')}
              </label>
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
                className="rounded-lg border border-stone-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 focus:border-primary-500 focus:outline-none focus:ring-1 focus:ring-primary-500">
                {customCloud.map(p => (
                  <option key={p.slug} value={`cloud:${p.slug}`}>
                    {p.label}
                  </option>
                ))}
                {localAvailable && <option value="local:">{t('settings.ai.localOllama')}</option>}
              </select>
            </div>

            <div className="flex flex-col gap-1.5">
              <label className="text-xs font-medium text-stone-700 dark:text-neutral-200">
                {t('settings.ai.modelLabel')}
              </label>
              {source?.kind === 'local' ? (
                <select
                  value={model}
                  onChange={e => setModel(e.target.value)}
                  className="rounded-lg border border-stone-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 focus:border-primary-500 focus:outline-none focus:ring-1 focus:ring-primary-500">
                  {localModels.map(m => (
                    <option key={m.id} value={m.id}>
                      {m.id}
                    </option>
                  ))}
                </select>
              ) : cloudModelsLoading ? (
                <select
                  disabled
                  className="rounded-lg border border-stone-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-400 dark:text-neutral-500 opacity-60 cursor-wait">
                  <option>Loading models…</option>
                </select>
              ) : cloudModelsError ? (
                <div className="space-y-1.5">
                  <div className="rounded-lg border border-red-200 dark:border-red-500/30 bg-red-50 dark:bg-red-500/10 px-3 py-2 text-xs text-red-700 dark:text-red-300 font-mono break-all">
                    {cloudModelsError}
                  </div>
                  <div className="flex items-center gap-2">
                    <button
                      type="button"
                      onClick={() => setModelsKey(k => k + 1)}
                      className="text-xs text-primary-600 dark:text-primary-400 hover:underline">
                      Retry
                    </button>
                    <span className="text-xs text-stone-400 dark:text-neutral-500">
                      or enter model id manually:
                    </span>
                  </div>
                  <input
                    type="text"
                    value={model}
                    onChange={e => setModel(e.target.value)}
                    placeholder={selectedCloud ? `${selectedCloud.slug} model id` : 'model-id'}
                    className="w-full rounded-lg border border-stone-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-2 text-sm font-mono text-stone-900 dark:text-neutral-100 placeholder-stone-400 dark:placeholder-neutral-500 focus:border-primary-500 focus:outline-none focus:ring-1 focus:ring-primary-500"
                  />
                </div>
              ) : cloudModels.length > 0 ? (
                <select
                  value={model}
                  onChange={e => setModel(e.target.value)}
                  className="rounded-lg border border-stone-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 focus:border-primary-500 focus:outline-none focus:ring-1 focus:ring-primary-500">
                  {!model && <option value="">Select a model…</option>}
                  {/* Keep existing value selectable even if the provider no longer lists it */}
                  {model && !cloudModels.some(m => m.id === model) && (
                    <option value={model}>{model}</option>
                  )}
                  {cloudModels.map(m => (
                    <option key={m.id} value={m.id}>
                      {humanizeModelId(m.id)} — {m.id}
                    </option>
                  ))}
                </select>
              ) : (
                <input
                  type="text"
                  value={model}
                  onChange={e => setModel(e.target.value)}
                  placeholder={selectedCloud ? `${selectedCloud.slug} model id` : 'model-id'}
                  className="rounded-lg border border-stone-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-2 text-sm font-mono text-stone-900 dark:text-neutral-100 placeholder-stone-400 dark:placeholder-neutral-500 focus:border-primary-500 focus:outline-none focus:ring-1 focus:ring-primary-500"
                />
              )}
            </div>

            {/* Temperature override (optional). When unchecked, the workload
                inherits the provider/global default temperature. */}
            <div className="flex flex-col gap-1.5">
              <label className="flex items-center justify-between gap-2 text-xs font-medium text-stone-700 dark:text-neutral-200">
                <span className="inline-flex items-center gap-2">
                  <input
                    type="checkbox"
                    checked={temperature != null}
                    onChange={e => setTemperature(e.target.checked ? 0.7 : null)}
                    className="h-3.5 w-3.5 rounded border-stone-300 dark:border-neutral-700 text-primary-500 focus:ring-primary-500"
                  />
                  Temperature override
                </span>
                {temperature != null && (
                  <span className="font-mono text-[11px] text-stone-500 dark:text-neutral-400">
                    {temperature.toFixed(2)}
                  </span>
                )}
              </label>
              {temperature != null && (
                <div className="flex items-center gap-2">
                  <input
                    type="range"
                    aria-label="Temperature override (slider)"
                    min={0}
                    max={2}
                    step={0.05}
                    value={temperature}
                    onChange={e => setTemperature(Number(e.target.value))}
                    className="flex-1 accent-primary-500"
                  />
                  <input
                    type="number"
                    aria-label="Temperature override (value)"
                    min={0}
                    max={2}
                    step={0.05}
                    value={temperature}
                    onChange={e => {
                      const v = Number(e.target.value);
                      if (Number.isFinite(v)) setTemperature(Math.max(0, Math.min(2, v)));
                    }}
                    className="w-16 rounded-lg border border-stone-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-2 py-1 text-xs font-mono text-stone-900 dark:text-neutral-100 focus:border-primary-500 focus:outline-none focus:ring-1 focus:ring-primary-500"
                  />
                </div>
              )}
              <p className="text-[11px] text-stone-400 dark:text-neutral-500">
                Lower = more deterministic. Leave unchecked to use the provider default.
              </p>
            </div>
          </div>
        )}

        <div className="mt-6 flex justify-end gap-2">
          <button
            type="button"
            onClick={onClose}
            className="rounded-lg border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-4 py-2 text-sm font-medium text-stone-700 dark:text-neutral-200 hover:bg-stone-50 dark:hover:bg-neutral-800/60 dark:bg-neutral-800/60 dark:hover:bg-neutral-800/60">
            {t('common.cancel')}
          </button>
          <button
            type="button"
            onClick={handleSave}
            disabled={!canSave}
            className="rounded-lg bg-primary-500 px-4 py-2 text-sm font-medium text-white hover:bg-primary-600 disabled:cursor-not-allowed disabled:opacity-50">
            {t('common.save')}
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
}) => {
  const { t } = useT();
  return (
    <div className="pointer-events-none sticky bottom-3 z-20 flex justify-center px-4">
      <div className="pointer-events-auto flex w-full items-center gap-2 rounded-lg border border-stone-200 dark:border-neutral-800 bg-white/95 dark:bg-neutral-900/95 px-3 py-2 shadow-float backdrop-blur-md animate-fade-up">
        <div className="flex h-6 w-6 shrink-0 items-center justify-center rounded bg-amber-50 dark:bg-amber-500/10 text-amber-600 dark:text-amber-300">
          <LuCircleAlert className="h-3.5 w-3.5" />
        </div>
        <div className="min-w-0 flex-1">
          <div className="text-xs font-medium text-stone-900 dark:text-neutral-100">
            {changeCount === 1
              ? t('settings.ai.unsavedChange')
              : `${String(changeCount)} ${t('settings.ai.unsavedChanges')}`}
          </div>
          <div className="truncate font-mono text-[10px] text-stone-500 dark:text-neutral-400">
            {diffSummary.slice(0, 2).join(' · ')}
            {diffSummary.length > 2 ? ` · +${diffSummary.length - 2}` : ''}
          </div>
        </div>
        <button
          onClick={onDiscard}
          className="rounded-md border border-stone-200 dark:border-neutral-800 px-2 py-1 text-xs font-medium text-stone-700 dark:text-neutral-200 hover:bg-stone-50 dark:hover:bg-neutral-800/60 dark:bg-neutral-800/60 dark:hover:bg-neutral-800/60">
          {t('settings.ai.discard')}
        </button>
        <button
          onClick={onSave}
          className="inline-flex items-center gap-1 rounded-md bg-primary-500 px-2.5 py-1 text-xs font-medium text-white hover:bg-primary-600">
          <LuCheck className="h-3 w-3" />
          {t('common.save')}
        </button>
      </div>
    </div>
  );
};

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
  const { t } = useT();
  const { navigateBack, breadcrumbs } = useSettingsNavigation();
  const { saved, draft, setDraft, isDirty, save, persist, discard, loading, error, reload } =
    useAISettings();
  // #1574 §4b: advisory re-embed modal, driven by the backend status RPC.
  // Logic lives in a unit-testable hook (see useReembedBackfillModal).
  const { reembed, handleSave, dismissReembed } = useReembedBackfillModal(save);
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
        const describe = (r: ProviderRef) => {
          if (r.kind === 'openhuman') return 'openhuman';
          const tempSuffix = r.temperature != null ? `@${r.temperature.toFixed(2)}` : '';
          if (r.kind === 'cloud') return `${r.providerSlug}:${r.model}${tempSuffix}`;
          return `local:${r.model}${tempSuffix}`;
        };
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
          <div className="border-b border-stone-200 dark:border-neutral-800 pb-2">
            <h2 className="text-base font-semibold text-stone-900 dark:text-neutral-100">
              {t('settings.ai.llmProviders')}
            </h2>
            <p className="text-xs text-stone-500 dark:text-neutral-400 mt-0.5">
              {t('settings.ai.llmProvidersDesc')}
            </p>
          </div>

          {/* ─── Provider chip-toggle list ────────────────────────────────── */}
          <section className="space-y-3">
            {loading && (
              <div className="text-xs text-stone-500 dark:text-neutral-400">
                {t('common.loading')}
              </div>
            )}
            {error && (
              <div className="rounded-md border border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 px-3 py-2 text-xs text-coral-700 dark:text-coral-300">
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
                      className={`relative inline-flex h-4 w-7 shrink-0 items-center rounded-full transition-colors disabled:cursor-wait disabled:opacity-60 ${enabled ? 'bg-primary-500' : 'bg-stone-300 dark:bg-neutral-700'}`}>
                      <span
                        aria-hidden
                        className={`inline-block h-3 w-3 transform rounded-full bg-white dark:bg-neutral-900 shadow transition-transform ${enabled ? 'translate-x-3.5' : 'translate-x-0.5'}`}
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
          <div className="border-b border-stone-200 dark:border-neutral-800 pb-2">
            <h2 className="text-base font-semibold text-stone-900 dark:text-neutral-100">
              {t('settings.ai.routing')}
            </h2>
            <p className="text-xs text-stone-500 dark:text-neutral-400 mt-0.5">
              {t('settings.ai.routingDesc')}
            </p>
          </div>

          <section className="space-y-3">
            <div className="overflow-hidden rounded-lg border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 px-3">
              <div className="pt-3">
                <div className="text-[10px] font-semibold uppercase tracking-wide text-stone-400 dark:text-neutral-500">
                  {t('settings.ai.workloadGroupChat')}
                </div>
                <div className="divide-y divide-stone-200 dark:divide-neutral-800">
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
                <div className="text-[10px] font-semibold uppercase tracking-wide text-stone-400 dark:text-neutral-500">
                  {t('settings.ai.workloadGroupBackground')}
                </div>
                <div className="divide-y divide-stone-200 dark:divide-neutral-800">
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

            <div className="text-[11px] text-stone-500 dark:text-neutral-400">
              {t('settings.ai.defaultResolvesTo')}{' '}
              <span className="font-mono text-stone-700 dark:text-neutral-200">OpenHuman</span>.
            </div>
          </section>
        </div>
        {/* end of Routing section */}

        <BackgroundLoopControls routing={draft.routing} cloudProviders={draft.cloudProviders} />
      </div>

      {isDirty && (
        <SaveBar
          diffSummary={diffSummary}
          changeCount={diffSummary.length}
          onSave={() => void handleSave()}
          onDiscard={discard}
        />
      )}

      <ConfirmationModal
        modal={{
          isOpen: reembed.open,
          title: 'Re-indexing memory',
          message:
            `Embeddings are being reprocessed. ${reembed.pending} memory item(s) ` +
            `are being re-embedded under the current model — semantic recall is ` +
            `reduced until this finishes. Keyword search keeps working, and ` +
            `re-embedding continues in the background if you close this.`,
          confirmText: 'OK',
          onConfirm: dismissReembed,
          onCancel: dismissReembed,
        }}
        onClose={dismissReembed}
      />

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

              // Snapshot the prior persisted cloud_providers list so we can
              // restore it if the live probe fails.
              const priorWireProviders = saved.cloudProviders.map(p => ({
                id: p.id,
                slug: p.slug,
                label: p.label,
                endpoint: p.endpoint,
                auth_style: p.authStyle,
              }));

              // Persist the credential BEFORE the probe so the factory has it
              // available. Let setCloudProviderKey throw — the editor's
              // button-click handler catches and surfaces the error inline.
              if (apiKey && upserted.slug !== 'openhuman') {
                await setCloudProviderKey(upserted.slug, apiKey);
              }

              // Live verification — flush the new cloud_providers list and
              // call `/models` through the Rust controller. Skip for the
              // OpenHuman backend (session JWT, no probe-able endpoint).
              if (upserted.slug !== 'openhuman') {
                const list =
                  editing === 'new'
                    ? [...draft.cloudProviders, upserted]
                    : draft.cloudProviders.map(p => (p.id === editing.id ? upserted : p));
                const nextWireProviders = list
                  .filter(p => !['', 'cloud', 'openhuman', 'pid'].includes(p.slug))
                  .map(p => ({
                    id: p.id,
                    slug: p.slug,
                    label: p.label,
                    endpoint: p.endpoint,
                    auth_style: p.authStyle,
                  }));
                await flushCloudProviders(nextWireProviders);
                try {
                  await listProviderModels(upserted.slug);
                } catch (probeErr) {
                  await flushCloudProviders(priorWireProviders).catch(() => {});
                  if (apiKey) {
                    await clearCloudProviderKey(upserted.slug).catch(() => {});
                  }
                  const msg = probeErr instanceof Error ? probeErr.message : String(probeErr);
                  throw new Error(`Could not reach ${upserted.label}: ${msg}`);
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
              onSubmit={async next => {
                const nextDraft = {
                  ...draft,
                  routing: { ...draft.routing, [customDialogFor]: next },
                };
                await persist(nextDraft);
                setCustomDialogFor(null);
              }}
            />
          );
        })()}

      {keyDialogFor && (
        <ProviderKeyDialog
          slug={keyDialogFor}
          label={pendingLocalLabel ?? BUILTIN_PROVIDER_META[keyDialogFor]?.label ?? keyDialogFor}
          isLocalRuntime={Boolean(pendingLocalLabel)}
          onCancel={() => {
            setKeyDialogFor(null);
            setPendingLocalLabel(null);
          }}
          onSubmit={async value => {
            const slug = keyDialogFor;
            const localLabel = pendingLocalLabel;
            const isLocalRuntime = Boolean(localLabel);
            setBusyAction(
              `toggle-${localLabel ? localLabel.toLowerCase().replace(/\s/g, '') : slug}`
            );
            try {
              const trimmed = value.trim();
              // Normalize local-runtime endpoints so the cloud_providers entry
              // always carries the OpenAI-compatible `/v1` path that the
              // `list_configured_models` probe expects. Without this,
              // `http://host:11434` would pass validation, be stored verbatim,
              // and silently fail model discovery until the user manually
              // appended `/v1` — confusing because the UI would still mark the
              // provider connected (caught in review).
              const endpoint = isLocalRuntime
                ? (() => {
                    const url = new URL(trimmed); // throws on malformed → caught above
                    if (!/^https?:$/.test(url.protocol)) {
                      throw new Error('Endpoint must start with http:// or https://');
                    }
                    if (url.pathname === '' || url.pathname === '/') {
                      url.pathname = '/v1';
                    }
                    return url.toString().replace(/\/$/, '');
                  })()
                : defaultEndpointFor(slug);
              const upserted: CloudProvider = {
                id: `p_${slug}_${Math.random().toString(36).slice(2, 7)}`,
                slug,
                label: localLabel ?? BUILTIN_PROVIDER_META[slug]?.label ?? slug,
                endpoint,
                authStyle: authStyleForSlug(slug),
                maskedKey: maskKeyLabel(true),
              };

              // Snapshot the prior persisted cloud_providers list so we can
              // roll back to it if the live probe fails. `saved` reflects what
              // is currently on disk (the eager-flush effect keeps it in sync
              // with `draft`), so this is the right baseline to restore to.
              const priorWireProviders = saved.cloudProviders.map(p => ({
                id: p.id,
                slug: p.slug,
                label: p.label,
                endpoint: p.endpoint,
                auth_style: p.authStyle,
              }));

              // Persist the credential / endpoint BEFORE the probe, so the
              // factory has everything it needs to actually answer it. Each
              // step short-circuits and surfaces its own error via throw —
              // ProviderKeyDialog.handleSave catches and keeps the dialog open
              // so the user can fix the value and retry.
              if (!isLocalRuntime && slug !== 'openhuman') {
                await setCloudProviderKey(slug, trimmed);
              } else if (isLocalRuntime && slug === 'ollama') {
                // The Rust Ollama branch reads `config.local_ai.base_url`
                // (not `cloud_providers[].endpoint`) when building the chat
                // provider — persist it eagerly so chat routing actually hits
                // the user-chosen host. Strip a trailing `/v1` since
                // `make_ollama_provider` appends `/v1` itself.
                const baseUrl = endpoint.replace(/\/v1\/?$/, '');
                await openhumanUpdateLocalAiSettings({
                  base_url: baseUrl,
                  provider: 'ollama',
                  runtime_enabled: true,
                  opt_in_confirmed: true,
                });
              }

              // Live verification: flush the new cloud_providers list to disk
              // and call `/models` through the Rust controller. A reachable
              // endpoint + valid auth header is the strongest check we can
              // make without burning tokens. Skip the probe for the
              // OpenHuman backend (session JWT, no /models endpoint to hit).
              if (slug !== 'openhuman') {
                const nextWireProviders = [
                  ...priorWireProviders.filter(p => p.slug !== slug),
                  {
                    id: upserted.id,
                    slug: upserted.slug,
                    label: upserted.label,
                    endpoint: upserted.endpoint,
                    auth_style: upserted.authStyle,
                  },
                ];
                await flushCloudProviders(nextWireProviders);
                try {
                  await listProviderModels(slug);
                } catch (probeErr) {
                  // Roll back so the UI / on-disk state never reflects a
                  // provider we couldn't actually reach. The user sees the
                  // error in the dialog and the chip stays in the OFF state.
                  await flushCloudProviders(priorWireProviders).catch(() => {});
                  if (!isLocalRuntime && slug !== 'openhuman') {
                    await clearCloudProviderKey(slug).catch(() => {});
                  }
                  const msg = probeErr instanceof Error ? probeErr.message : String(probeErr);
                  throw new Error(`Could not reach ${upserted.label}: ${msg}`);
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
  const { t } = useT();
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
  const [submitError, setSubmitError] = useState<string | null>(null);
  const isOpenHuman = slug === 'openhuman';
  const hasExistingKey = (initial?.maskedKey ?? '').startsWith('••••');

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-stone-900/30 p-4">
      <div className="w-full max-w-md rounded-lg border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 shadow-float">
        <div className="border-b border-stone-200 dark:border-neutral-800 px-4 py-3">
          <div className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
            {initial
              ? `${t('settings.ai.editProvider')} ${initial.label}`
              : t('settings.ai.addCloudProvider')}
          </div>
          <div className="mt-0.5 text-xs text-stone-500 dark:text-neutral-400">
            {t('settings.ai.apiKeysEncrypted')}{' '}
            <span className="font-mono">auth-profiles.json</span>.
          </div>
        </div>
        <div className="space-y-3 px-4 py-3">
          <div>
            <label className="text-[10px] font-semibold uppercase tracking-wide text-stone-500 dark:text-neutral-400">
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
              className="mt-1 w-full rounded-lg border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 disabled:opacity-60 focus:border-primary-400 focus:outline-none focus:ring-1 focus:ring-primary-200">
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
            <label className="text-[10px] font-semibold uppercase tracking-wide text-stone-500 dark:text-neutral-400">
              Display label
            </label>
            <input
              value={label}
              onChange={e => setLabel(e.target.value)}
              className="mt-1 w-full rounded-lg border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 placeholder:text-stone-400 dark:placeholder:text-neutral-500 dark:text-neutral-500 dark:placeholder:text-neutral-500 focus:border-primary-400 focus:outline-none focus:ring-1 focus:ring-primary-200"
              placeholder="My Provider"
            />
          </div>
          <div>
            <label className="text-[10px] font-semibold uppercase tracking-wide text-stone-500 dark:text-neutral-400">
              Endpoint
            </label>
            <input
              value={endpoint}
              onChange={e => setEndpoint(e.target.value)}
              disabled={isOpenHuman}
              className="mt-1 w-full rounded-lg border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-3 py-2 font-mono text-xs text-stone-900 dark:text-neutral-100 placeholder:text-stone-400 dark:placeholder:text-neutral-500 dark:text-neutral-500 dark:placeholder:text-neutral-500 disabled:opacity-60 focus:border-primary-400 focus:outline-none focus:ring-1 focus:ring-primary-200"
              placeholder="https://api.example.com/v1"
            />
          </div>
          {!isOpenHuman && (
            <div>
              <label className="flex items-center justify-between text-[10px] font-semibold uppercase tracking-wide text-stone-500 dark:text-neutral-400">
                <span>API key</span>
                {hasExistingKey && (
                  <button
                    onClick={() => void onClearKey(slug)}
                    className="text-[10px] font-medium normal-case text-coral-600 dark:text-coral-300 hover:text-coral-700 dark:text-coral-300">
                    {t('settings.ai.clearStoredKey')}
                  </button>
                )}
              </label>
              <input
                type="password"
                value={apiKey}
                onChange={e => setApiKey(e.target.value)}
                className="mt-1 w-full rounded-lg border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-3 py-2 font-mono text-xs text-stone-900 dark:text-neutral-100 placeholder:text-stone-400 dark:placeholder:text-neutral-500 dark:text-neutral-500 dark:placeholder:text-neutral-500 focus:border-primary-400 focus:outline-none focus:ring-1 focus:ring-primary-200"
                placeholder={hasExistingKey ? 'Leave blank to keep existing key' : 'sk-...'}
              />
            </div>
          )}
          {submitError && (
            <div className="rounded-md border border-red-200 dark:border-red-500/30 bg-red-50 dark:bg-red-500/10 px-3 py-2 text-xs text-red-700 dark:text-red-300 break-words">
              {submitError}
            </div>
          )}
        </div>
        <div className="flex items-center justify-end gap-2 border-t border-stone-200 dark:border-neutral-800 px-4 py-3">
          <button
            onClick={onClose}
            disabled={saving}
            className="rounded-lg border border-stone-200 dark:border-neutral-800 px-3 py-1.5 text-xs font-medium text-stone-700 dark:text-neutral-200 hover:bg-stone-50 dark:hover:bg-neutral-800/60 dark:bg-neutral-800/60 dark:hover:bg-neutral-800/60 disabled:opacity-50">
            {t('common.cancel')}
          </button>
          <button
            onClick={async () => {
              setSaving(true);
              setSubmitError(null);
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
              } catch (err) {
                // Caller throws when the live /models probe rejects — surface
                // the failure inline and keep the dialog open so the user can
                // fix the key/URL and retry.
                setSubmitError(err instanceof Error ? err.message : String(err));
              } finally {
                setSaving(false);
              }
            }}
            disabled={saving || !endpoint.trim()}
            className="rounded-lg bg-primary-500 px-3 py-1.5 text-xs font-medium text-white hover:bg-primary-600 disabled:opacity-50">
            {saving
              ? t('settings.ai.saving')
              : initial
                ? t('settings.ai.saveChanges')
                : t('settings.ai.addProvider')}
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
    case 'ollama':
      // Ollama exposes an OpenAI-compatible endpoint at /v1; the bare host is
      // also accepted by the Rust factory (it appends /v1 internally for chat).
      // For the /models probe we want the OpenAI-compat path.
      return 'http://localhost:11434/v1';
    case 'lmstudio':
      return 'http://localhost:1234/v1';
    default:
      return '';
  }
}

export default AIPanel;
