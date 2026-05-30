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
import { LuCheck, LuCircleAlert, LuKeyRound } from 'react-icons/lu';

import { listConnections as listComposioConnections } from '../../../lib/composio/composioApi';
import type { ComposioConnection } from '../../../lib/composio/types';
import { useT } from '../../../lib/i18n/I18nContext';
import {
  type AISettings as ApiAISettings,
  type ProviderRef as ApiProviderRef,
  clearCloudProviderKey,
  type CloudProviderView,
  flushCloudProviders,
  importOpenAiCodexCliAuth,
  listProviderModels,
  loadAISettings,
  loadLocalProviderSnapshot,
  type LocalProviderSnapshot,
  type ModelInfo,
  OPENAI_CODEX_OAUTH_MISSING_AUTH_URL,
  OPENAI_CODEX_OAUTH_MISSING_CALLBACK_URL,
  saveAISettings,
  setCloudProviderKey,
  testProviderModel,
} from '../../../services/api/aiSettingsApi';
import {
  creditsApi,
  type CreditTransaction,
  type TeamUsage,
} from '../../../services/api/creditsApi';
import { connectOpenRouterViaOAuth } from '../../../utils/openrouterOAuth';
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
import { presentProviderSetupError, ProviderSetupErrorNotice } from './ProviderSetupErrorNotice';
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
  | 'heartbeat'
  | 'learning'
  | 'subconscious';

type WorkloadGroup = 'chat' | 'background';

type ProviderRef =
  | { kind: 'openhuman' }
  | { kind: 'default' }
  | { kind: 'cloud'; providerSlug: string; model: string; temperature?: number | null }
  | { kind: 'local'; model: string; temperature?: number | null };

type Workload = { id: WorkloadId; group: WorkloadGroup; label: string; description: string };

type RoutingMap = Record<WorkloadId, ProviderRef>;
type RoutingMode = 'managed' | 'own' | 'custom';
const ROUTING_WORKLOAD_IDS: WorkloadId[] = [
  'chat',
  'reasoning',
  'agentic',
  'coding',
  'memory',
  'heartbeat',
  'learning',
  'subconscious',
];

// ─────────────────────────────────────────────────────────────────────────────
// Static catalog
// ─────────────────────────────────────────────────────────────────────────────

// Slug-keyed display metadata for built-in provider slugs. Used only for
// chip rendering (label, tone). Custom providers use `provider.label` directly.
const BUILTIN_PROVIDER_META: Record<string, { tone: string; label: string }> = {
  openhuman: {
    label: 'Managed',
    tone: 'bg-emerald-50 dark:bg-emerald-500/10 ring-emerald-200 text-emerald-900 dark:text-emerald-100',
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
  orcarouter: {
    label: 'OrcaRouter',
    tone: 'bg-sky-50 dark:bg-sky-500/10 ring-sky-200 text-sky-900 dark:text-sky-100',
  },
  gmi: {
    label: 'GMI',
    tone: 'bg-fuchsia-50 dark:bg-fuchsia-500/10 ring-fuchsia-200 text-fuchsia-900 dark:text-fuchsia-100',
  },
  fireworks: {
    label: 'Fireworks',
    tone: 'bg-rose-50 dark:bg-rose-500/10 ring-rose-200 text-rose-900 dark:text-rose-100',
  },
  moonshot: {
    label: 'Kimi (Moonshot)',
    tone: 'bg-indigo-50 dark:bg-indigo-500/10 ring-indigo-200 text-indigo-900 dark:text-indigo-100',
  },
  custom: {
    label: 'Advanced',
    tone: 'bg-sky-50 dark:bg-sky-500/10 ring-sky-200 text-sky-900 dark:text-sky-100',
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

const WORKLOAD_MODEL_HINTS: Record<WorkloadId, string> = {
  chat: 'Recommended: a cheap or mid-cost fast chat model with high tokens/sec and low latency. Open-source local models can work well here if they feel responsive.',
  reasoning:
    'Recommended: a more expensive frontier or strong reasoning model for deep thinking. This is used for the main chat agent, meeting summaries, and heavier answer synthesis.',
  agentic:
    'Recommended: a reliable instruction-following model with strong tool use. Mid-cost frontier models are usually safest; capable open-source models can work if tool calling is stable.',
  coding:
    'Recommended: a coding-tuned model with strong instruction following, edit quality, and long-context performance. This is usually worth spending more on.',
  memory:
    'Recommended: a cheaper summarization model. It should be consistent and compact, but it does not need premium frontier-level reasoning.',
  heartbeat:
    'Recommended: a cheap, efficient background model. This runs often between turns, so low cost matters more than maximum intelligence.',
  learning:
    'Recommended: a stronger reflective model. This can be mid-cost or premium because it benefits from better synthesis over recent history.',
  subconscious:
    'Recommended: a very cheap monitoring model, ideally one that is lightweight and predictable. This is for eventfulness scoring, drift checks, and quiet background evaluation.',
};

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
  chat: { kind: 'default' },
  reasoning: { kind: 'default' },
  agentic: { kind: 'default' },
  coding: { kind: 'default' },
  memory: { kind: 'default' },
  heartbeat: { kind: 'default' },
  learning: { kind: 'default' },
  subconscious: { kind: 'default' },
};

const EMPTY_SETTINGS: AISettings = { cloudProviders: [], routing: EMPTY_ROUTING };

function maskKeyLabel(hasKey: boolean): string {
  return hasKey ? '•••• configured' : 'Not configured';
}

function slugifyCustomProviderName(name: string): string {
  return name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '');
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

function providerToggleAriaLabel(
  t: (key: string, fallback?: string) => string,
  enabled: boolean,
  label: string
): string {
  return formatI18n(
    enabled ? t('settings.ai.disconnectProvider') : t('settings.ai.connectProviderLabel'),
    { label }
  );
}

function formatI18n(template: string, vars: Record<string, string | number>): string {
  return Object.entries(vars).reduce(
    (result, [key, value]) => result.replaceAll(`{${key}}`, String(value)),
    template
  );
}

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
  locked = false,
  onToggle,
}: {
  slug: string;
  label: string;
  enabled: boolean;
  busy?: boolean;
  locked?: boolean;
  onToggle: () => void;
}) => {
  const { t } = useT();
  const tone = slugTone(slug);
  return (
    <div
      className={`inline-flex items-center gap-2 rounded-full px-2.5 py-1 text-xs font-medium ring-1 transition-colors dark:ring-neutral-700 ${tone}`}>
      <span>{label}</span>
      <button
        type="button"
        role="switch"
        aria-checked={enabled}
        aria-label={providerToggleAriaLabel(t, enabled, label)}
        disabled={busy || locked}
        onClick={onToggle}
        className={`relative inline-flex h-4 w-7 shrink-0 items-center rounded-full transition-colors disabled:cursor-not-allowed disabled:opacity-60 ${enabled ? 'bg-primary-500' : 'bg-stone-300 dark:bg-neutral-700'}`}>
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
  oauthAction,
  onCancel,
  onSubmit,
}: {
  slug: string;
  label: string;
  /** When true, render an "Endpoint URL" field instead of API key. */
  isLocalRuntime: boolean;
  oauthAction?: { label: string; description?: string; onClick: () => Promise<void> | void } | null;
  onCancel: () => void;
  /** Returns the entered value. For local runtimes this is the endpoint URL;
   *  for cloud providers it's the API key. */
  onSubmit: (value: string) => Promise<void> | void;
}) => {
  const { t } = useT();
  const [value, setValue] = useState<string>(isLocalRuntime ? defaultEndpointFor(slug) : '');
  const [phase, setPhase] = useState<'idle' | 'saving' | 'oauth'>('idle');
  const [error, setError] = useState<string | null>(null);
  const busy = phase !== 'idle';

  const placeholder = isLocalRuntime
    ? defaultEndpointFor(slug) || t('settings.ai.defaultLocalEndpoint')
    : slug === 'openai'
      ? 'sk-...'
      : slug === 'anthropic'
        ? 'sk-ant-...'
        : slug === 'openrouter'
          ? 'sk-or-...'
          : slug === 'orcarouter'
            ? 'sk-orca-...'
            : slug === 'gmi'
              ? 'gmi-...'
              : slug === 'fireworks'
                ? 'fw-...'
                : slug === 'moonshot'
                  ? 'sk-...'
                  : 'your-api-key';

  const fieldLabel = isLocalRuntime
    ? t('settings.ai.endpointUrlLabel')
    : t('settings.ai.apiKeyFieldLabel');
  const helper = isLocalRuntime
    ? formatI18n(t('settings.ai.localRuntimeHelper'), { label })
    : t('settings.ai.apiKeyStoredEncrypted');

  const handleSave = async () => {
    const trimmed = value.trim();
    if (!trimmed) {
      setError(
        isLocalRuntime ? t('settings.ai.endpointUrlRequired') : t('settings.ai.apiKeyRequired')
      );
      return;
    }
    if (isLocalRuntime && !/^https?:\/\//i.test(trimmed)) {
      setError(t('settings.ai.endpointProtocolRequired'));
      return;
    }
    setError(null);

    setPhase('saving');
    try {
      await onSubmit(trimmed);
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      console.warn('[ai-settings] provider setup failed', {
        slug,
        local_runtime: isLocalRuntime,
        summary: presentProviderSetupError(message, t).summary,
      });
      setError(message);
      setPhase('idle');
    }
  };

  const handleOAuth = async () => {
    if (!oauthAction) return;
    setError(null);
    setPhase('oauth');
    try {
      await oauthAction.onClick();
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      console.warn('[ai-settings] provider oauth failed', {
        slug,
        summary: presentProviderSetupError(message, t).summary,
      });
      setError(message);
      setPhase('idle');
    }
  };

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label={formatI18n(t('settings.ai.connectProviderDialog'), { label })}
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
          {error ? <ProviderSetupErrorNotice error={error} /> : null}
        </div>

        {oauthAction ? (
          <div className="mt-4 rounded-xl border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/50 p-3">
            <div className="text-[11px] font-semibold uppercase tracking-wide text-stone-500 dark:text-neutral-400">
              {t('settings.ai.or')}
            </div>
            <p className="mt-1 text-xs text-stone-500 dark:text-neutral-400">
              {oauthAction.description ?? t('settings.ai.openRouterOauthDescription')}
            </p>
            <button
              type="button"
              onClick={() => void handleOAuth()}
              disabled={busy}
              className="mt-3 inline-flex items-center justify-center rounded-lg border border-stone-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-4 py-2 text-sm font-medium text-stone-900 dark:text-neutral-100 hover:bg-stone-100 dark:hover:bg-neutral-800 disabled:cursor-not-allowed disabled:opacity-50">
              {phase === 'oauth' ? t('settings.ai.connecting') : oauthAction.label}
            </button>
          </div>
        ) : null}

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

function describeProvider(ref: ProviderRef, providers: BackgroundLoopProviderView[]): string {
  if (ref.kind === 'openhuman') return 'Managed · OpenHuman';
  if (ref.kind === 'default') return 'Default route';
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
  <div className="min-w-0 overflow-hidden rounded-md bg-stone-50 dark:bg-neutral-800/60 px-3 py-2">
    <div className="truncate text-[10px] font-semibold uppercase tracking-wide text-stone-400 dark:text-neutral-500">
      {label}
    </div>
    <div className="mt-1 truncate text-sm font-semibold text-stone-900 dark:text-neutral-100">
      {value}
    </div>
    {detail ? (
      <div className="mt-0.5 truncate text-[11px] text-stone-500 dark:text-neutral-400">
        {detail}
      </div>
    ) : null}
  </div>
);

const FormulaRow = ({ label, value, detail }: { label: string; value: string; detail: string }) => (
  <div className="min-w-0 overflow-hidden rounded-md border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-3 py-2">
    <div className="flex items-center justify-between gap-3">
      <span className="min-w-0 truncate text-xs font-medium text-stone-800 dark:text-neutral-100">
        {label}
      </span>
      <span className="shrink-0 font-mono text-xs text-stone-600 dark:text-neutral-300">
        {value}
      </span>
    </div>
    <div className="mt-1 truncate text-[11px] text-stone-500 dark:text-neutral-400">{detail}</div>
  </div>
);

export type BackgroundLoopControlsView = 'all' | 'heartbeat' | 'ledger';

/** Minimal cloud-provider shape consumed by the loop map's `describeProvider`
 *  helper — only slug/label/id are read. Accepting this narrower shape lets
 *  external panels (HeartbeatPanel, LedgerUsagePanel) feed in the API view
 *  (`CloudProviderView`) without copying the AIPanel-internal extras
 *  (`authStyle`, `maskedKey`). */
export type BackgroundLoopProviderView = { id: string; slug: string; label: string };

export const BackgroundLoopControls = ({
  routing,
  cloudProviders,
  view = 'all',
  hideHeader = false,
}: {
  routing: RoutingMap;
  cloudProviders: BackgroundLoopProviderView[];
  view?: BackgroundLoopControlsView;
  hideHeader?: boolean;
}) => {
  const { t } = useT();
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

  const showHeartbeat = view === 'all' || view === 'heartbeat';
  const showLedger = view === 'all' || view === 'ledger';
  const gridCols =
    view === 'all' ? 'md:grid-cols-[minmax(0,1fr)_minmax(260px,0.8fr)]' : 'grid-cols-1';

  return (
    <div className="space-y-4">
      {!hideHeader && (
        <div className="border-b border-stone-200 dark:border-neutral-800 pb-2">
          <h2 className="text-base font-semibold text-stone-900 dark:text-neutral-100">
            {t('settings.ai.backgroundLoops')}
          </h2>
          <p className="mt-0.5 text-xs text-stone-500 dark:text-neutral-400">
            {t('settings.ai.backgroundLoopsDesc')}
          </p>
        </div>
      )}

      {error && (
        <div className="rounded-md border border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 px-3 py-2 text-xs text-coral-700 dark:text-coral-300">
          {error}
        </div>
      )}

      <section className={`grid gap-3 ${gridCols}`}>
        {showHeartbeat && (
          <div className="space-y-3">
            <div className="rounded-lg border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 p-3">
              <div className="mb-3 flex items-center justify-between gap-3">
                <div>
                  <div className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
                    {t('settings.ai.heartbeatControls')}
                  </div>
                  <div className="text-xs text-stone-500 dark:text-neutral-400">
                    {t('settings.ai.heartbeatControlsDesc')}
                  </div>
                </div>
                <button
                  type="button"
                  onClick={() => void refresh()}
                  disabled={loading}
                  className="rounded-md border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-2 py-1 text-xs font-medium text-stone-700 dark:text-neutral-200 hover:bg-stone-50 dark:hover:bg-neutral-800/60 dark:bg-neutral-800/60 dark:hover:bg-neutral-800/60 disabled:opacity-50">
                  {t('common.refresh')}
                </button>
              </div>

              {settings ? (
                <div className="space-y-2">
                  <LoopToggle
                    label={t('settings.ai.heartbeatLoop')}
                    description={t('settings.ai.heartbeatLoopDesc')}
                    checked={settings.enabled}
                    busy={saving === 'enabled'}
                    onToggle={() => void applyHeartbeatPatch({ enabled: !settings.enabled })}
                  />
                  <LoopToggle
                    label={t('settings.ai.subconsciousInference')}
                    description={t('settings.ai.subconsciousInferenceDesc')}
                    checked={settings.inference_enabled}
                    busy={saving === 'inference_enabled'}
                    onToggle={() =>
                      void applyHeartbeatPatch({ inference_enabled: !settings.inference_enabled })
                    }
                  />
                  <LoopToggle
                    label={t('settings.ai.calendarMeetingChecks')}
                    description={t('settings.ai.calendarMeetingChecksDesc')}
                    checked={settings.notify_meetings}
                    busy={saving === 'notify_meetings'}
                    onToggle={() =>
                      void applyHeartbeatPatch({ notify_meetings: !settings.notify_meetings })
                    }
                  />
                  <div className="grid gap-2 rounded-lg border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-3 py-2 sm:grid-cols-3">
                    <label className="min-w-0 space-y-1 text-xs font-medium text-stone-700 dark:text-neutral-200">
                      <span className="whitespace-nowrap">{t('settings.ai.calendarCap')}</span>
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
                            {formatI18n(t('settings.ai.connectionsPerTick'), { count })}
                          </option>
                        ))}
                      </select>
                    </label>
                    <label className="min-w-0 space-y-1 text-xs font-medium text-stone-700 dark:text-neutral-200">
                      <span className="whitespace-nowrap">{t('settings.ai.meetingLookahead')}</span>
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
                            {formatI18n(t('settings.ai.minutesShort'), { count: minutes })}
                          </option>
                        ))}
                      </select>
                    </label>
                    <label className="min-w-0 space-y-1 text-xs font-medium text-stone-700 dark:text-neutral-200">
                      <span className="whitespace-nowrap">
                        {t('settings.ai.reminderLookahead')}
                      </span>
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
                            {formatI18n(t('settings.ai.minutesShort'), { count: minutes })}
                          </option>
                        ))}
                      </select>
                    </label>
                  </div>
                  <LoopToggle
                    label={t('settings.ai.cronReminderChecks')}
                    description={t('settings.ai.cronReminderChecksDesc')}
                    checked={settings.notify_reminders}
                    busy={saving === 'notify_reminders'}
                    onToggle={() =>
                      void applyHeartbeatPatch({ notify_reminders: !settings.notify_reminders })
                    }
                  />
                  <LoopToggle
                    label={t('settings.ai.relevantNotificationChecks')}
                    description={t('settings.ai.relevantNotificationChecksDesc')}
                    checked={settings.notify_relevant_events}
                    busy={saving === 'notify_relevant_events'}
                    onToggle={() =>
                      void applyHeartbeatPatch({
                        notify_relevant_events: !settings.notify_relevant_events,
                      })
                    }
                  />
                  <LoopToggle
                    label={t('settings.ai.externalDelivery')}
                    description={t('settings.ai.externalDeliveryDesc')}
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
                      {t('settings.ai.interval')}
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
                          {formatI18n(t('settings.ai.minutesShort'), { count: minutes })}
                        </option>
                      ))}
                    </select>
                    <button
                      type="button"
                      onClick={() => void runPlannerNow()}
                      disabled={runningTick}
                      className="ml-auto rounded-md border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-2 py-1 text-xs font-medium text-stone-700 dark:text-neutral-200 hover:bg-stone-50 dark:hover:bg-neutral-800/60 dark:bg-neutral-800/60 dark:hover:bg-neutral-800/60 disabled:opacity-50">
                      {runningTick ? t('settings.ai.running') : t('settings.ai.plannerTickNow')}
                    </button>
                  </div>

                  {plannerSummary && (
                    <div className="rounded-md border border-primary-100 bg-primary-50 dark:bg-primary-500/10 px-3 py-2 text-xs text-primary-900">
                      {t('settings.ai.plannerSummary')
                        .replace('{sourceEvents}', String(plannerSummary.source_events))
                        .replace('{sent}', String(plannerSummary.deliveries_sent))
                        .replace('{deduped}', String(plannerSummary.deliveries_skipped_dedup))}
                    </div>
                  )}
                </div>
              ) : (
                <div className="text-xs text-stone-500 dark:text-neutral-400">
                  {loading
                    ? t('settings.ai.loadingHeartbeatControls')
                    : t('settings.ai.heartbeatControlsUnavailable')}
                </div>
              )}
            </div>

            <div className="overflow-hidden rounded-lg border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60">
              <div className="border-b border-stone-200 dark:border-neutral-800 px-3 py-2 text-xs font-semibold uppercase tracking-wide text-stone-400 dark:text-neutral-500">
                {t('settings.ai.loopMap')}
              </div>
              <div className="divide-y divide-stone-200 dark:divide-neutral-800">
                {loops.map(loop => (
                  <div key={loop.name} className="grid gap-2 px-3 py-3 md:grid-cols-[150px_1fr]">
                    <div className="min-w-0">
                      <div className="truncate text-sm font-medium text-stone-900 dark:text-neutral-100">
                        {loop.name}
                      </div>
                      <div className="mt-0.5 flex flex-wrap gap-1 text-[11px] text-stone-500 dark:text-neutral-400">
                        <span>{loop.enabled ? t('settings.ai.on') : t('settings.ai.off')}</span>
                        <span>{loop.cadence}</span>
                      </div>
                    </div>
                    <div className="min-w-0 text-xs text-stone-600 dark:text-neutral-300">
                      <div>{loop.work}</div>
                      <div className="mt-1 font-mono text-[11px] text-stone-500 dark:text-neutral-400">
                        {t('settings.ai.routeLabel').replace('{route}', loop.route)}
                      </div>
                      <div className="mt-1 text-stone-500 dark:text-neutral-400">{loop.risk}</div>
                    </div>
                  </div>
                ))}
              </div>
            </div>
          </div>
        )}

        {showLedger && (
          <div className="rounded-lg border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 p-3">
            <div className="flex items-center justify-between gap-3">
              <div>
                <div className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
                  {t('settings.ai.recentUsageLedger')}
                </div>
                <div className="text-xs text-stone-500 dark:text-neutral-400">
                  {t('settings.ai.recentUsageLedgerDesc')}
                </div>
              </div>
              <button
                type="button"
                onClick={() => void refresh()}
                disabled={loading}
                className="rounded-md border border-stone-200 dark:border-neutral-800 px-2 py-1 text-xs font-medium text-stone-700 dark:text-neutral-200 hover:bg-stone-50 dark:hover:bg-neutral-800/60 dark:bg-neutral-800/60 dark:hover:bg-neutral-800/60 disabled:opacity-50">
                {t('common.reload')}
              </button>
            </div>

            <div className="mt-3 grid grid-cols-2 gap-2 md:grid-cols-3">
              <MetricTile
                label={t('settings.ai.weekBudget')}
                value={usage ? formatUsd(usage.cycleBudgetUsd) : 'n/a'}
                detail={`resets ${formatDateTime(usage?.cycleEndsAt)}`}
              />
              <MetricTile
                label={t('settings.ai.cycleRemaining')}
                value={usage ? formatUsd(usage.remainingUsd) : 'n/a'}
                detail={usage ? `${formatUsd(usage.cycleSpentUsd)} used` : undefined}
              />
              <MetricTile
                label={t('settings.ai.cycleTotalSpend')}
                value={usage ? formatUsd(usage.insights.totals.totalUsd) : 'n/a'}
                detail={
                  usage
                    ? `inference ${formatUsd(usage.insights.totals.inferenceUsd)} + integrations ${formatUsd(usage.insights.totals.integrationsUsd)}`
                    : undefined
                }
              />
              <MetricTile
                label={t('settings.ai.avgSpendRow')}
                value={spendSample.avgRowUsd > 0 ? formatUsd(spendSample.avgRowUsd) : 'n/a'}
                detail={`${spendRows.length} recent spend rows`}
              />
              <MetricTile
                label={t('settings.ai.backgroundApiReads')}
                value={`${formatCount(backgroundApiReadsPerWeek)}/week`}
                detail={`${formatCount(calendarPlannerCallsPerWeek)} planner + ${formatCount(composioConnectionScansPerWeek)} sync`}
              />
              <MetricTile
                label={t('settings.ai.backgroundWakeups')}
                value={`${formatCount(backgroundWakeupsPerWeek)}/week`}
                detail={`${formatCount(memoryPollsPerWeek)} memory polls`}
              />
            </div>

            <div className="mt-3 rounded-lg border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 p-3">
              <div className="text-[10px] font-semibold uppercase tracking-wide text-stone-400 dark:text-neutral-500">
                {t('settings.ai.budgetMath')}
              </div>
              <div className="mt-2 grid gap-2">
                <FormulaRow
                  label={t('settings.ai.rowsLeft')}
                  value={estimatedRowsLeft !== null ? formatCount(estimatedRowsLeft) : 'n/a'}
                  detail={
                    estimatedRowsLeft !== null
                      ? `remaining / avg row = ${formatUsd(usage?.remainingUsd ?? 0)} / ${formatUsd(spendSample.avgRowUsd)}`
                      : 'Need recent spend rows to estimate.'
                  }
                />
                <FormulaRow
                  label={t('settings.ai.rowsPerFullWeekBudget')}
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
                  label={t('settings.ai.sampleBurnRate')}
                  value={
                    spendSample.spendPerHour > 0
                      ? `${formatUsd(spendSample.spendPerHour)}/hr`
                      : 'n/a'
                  }
                  detail={
                    spendSample.sampleHours > 0
                      ? `${formatCount(spendSample.rowsPerHour)} rows/hr across ${spendSample.sampleHours.toFixed(1)}h sample`
                      : 'Need timestamps from at least two spend rows.'
                  }
                />
                <FormulaRow
                  label={t('settings.ai.projectedEmpty')}
                  value={projectedExhaustAt}
                  detail={
                    projectedHoursLeft !== null
                      ? `${projectedHoursLeft.toFixed(1)}h after latest spend at recent burn rate`
                      : 'No projection without recent hourly spend.'
                  }
                />
                <FormulaRow
                  label={t('settings.ai.apiReadsPerDollarRemaining')}
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
                {t('settings.ai.loopCallBudget')}
              </div>
              <div className="mt-2 grid gap-2">
                <FormulaRow
                  label={t('settings.ai.heartbeatTicks')}
                  value={`${formatCount(heartbeatTicksPerWeek)}/week`}
                  detail={`10080 min/week / ${heartbeatIntervalMinutes} min interval`}
                />
                <FormulaRow
                  label={t('settings.ai.calendarPlannerCalls')}
                  value={`${formatCount(calendarPlannerCallsPerWeek)}/week`}
                  detail={
                    settings?.notify_meetings
                      ? `ticks * (1 list_connections + ${calendarConnectionsPolled} GOOGLECALENDAR_EVENTS_LIST)`
                      : 'Meeting collector disabled.'
                  }
                />
                <FormulaRow
                  label={t('settings.ai.calendarFanoutCap')}
                  value={`${formatCount(calendarConnectionsPolled)}/${formatCount(activeCalendarConnections.length)} conn/tick`}
                  detail={`max_calendar_connections_per_tick = ${maxCalendarConnectionsPerTick}; skipped now = ${calendarConnectionsSkipped}`}
                />
                <FormulaRow
                  label={t('settings.ai.subconsciousModelCalls')}
                  value={`${formatCount(subconsciousModelCallsPerWeek)}/week`}
                  detail={
                    settings?.enabled && settings.inference_enabled
                      ? 'one kind=subconscious_tick model call per heartbeat tick'
                      : 'Heartbeat inference disabled.'
                  }
                />
                <FormulaRow
                  label={t('settings.ai.composioSyncScans')}
                  value={`${formatCount(composioConnectionScansPerWeek)}/week`}
                  detail={`${activeConnections.length} active integration connection(s) scanned every ${COMPOSIO_PERIODIC_TICK_MINUTES} min`}
                />
                <FormulaRow
                  label={t('settings.ai.totalBackgroundApiReadBudget')}
                  value={`${formatCount(backgroundApiReadsPerWeek)}/week`}
                  detail={`calendar planner reads + periodic integration scans; excludes user-initiated chat tools`}
                />
                <FormulaRow
                  label={t('settings.ai.memoryWorkerPolls')}
                  value={`${formatCount(memoryPollsPerWeek)}/week max`}
                  detail={`${MEMORY_WORKERS} workers * ${MEMORY_POLL_SECONDS}s poll; LLM calls only for queued jobs`}
                />
              </div>
            </div>

            {latestSpend && (
              <div className="mt-3 rounded-md border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 px-3 py-2 text-xs text-stone-600 dark:text-neutral-300">
                {t('settings.ai.latestSpend')
                  .replace('{amount}', formatUsd(spendAmount(latestSpend)))
                  .replace('{time}', new Date(latestSpend.createdAt).toLocaleString())
                  .replace('{action}', latestSpend.action)}
              </div>
            )}

            <div className="mt-3 space-y-3">
              <div>
                <div className="text-[10px] font-semibold uppercase tracking-wide text-stone-400 dark:text-neutral-500">
                  {t('settings.ai.topActions')}
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
                      {t('settings.ai.noSpendRows')}
                    </div>
                  )}
                </div>
              </div>

              <div>
                <div className="text-[10px] font-semibold uppercase tracking-wide text-stone-400 dark:text-neutral-500">
                  {t('settings.ai.topHours')}
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
                      {t('settings.ai.noHourlySpend')}
                    </div>
                  )}
                </div>
              </div>
            </div>
          </div>
        )}
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

type WorkloadRowProps = { workload: Workload; ref_: ProviderRef; cloudProviders: CloudProvider[] };

const WorkloadRow = ({
  workload,
  ref_,
  cloudProviders,
  onCustomClick,
}: WorkloadRowProps & { onCustomClick: () => void }) => {
  const { t } = useT();
  const selectedCloud =
    ref_.kind === 'cloud' ? cloudProviders.find(c => c.slug === ref_.providerSlug) : undefined;
  const isCustom = ref_.kind === 'cloud' || ref_.kind === 'local';

  let resolved = '';
  if (ref_.kind === 'cloud') {
    resolved = selectedCloud
      ? `${selectedCloud.label} · ${ref_.model}`
      : `${ref_.providerSlug} · ${ref_.model}`;
  } else if (ref_.kind === 'local') {
    resolved = formatI18n(t('settings.ai.localModelResolved'), { model: ref_.model });
  } else if (ref_.kind === 'openhuman') {
    resolved = t('settings.ai.openhumanDefault');
  }

  return (
    <div className="flex items-center justify-between gap-3 py-3 transition-colors">
      <div className="min-w-0 flex-1 space-y-1">
        <div className="text-sm font-medium text-stone-900 dark:text-neutral-100">
          {workload.label}
        </div>
        <div className="text-xs leading-5 text-stone-500 dark:text-neutral-400">
          {workload.description}
        </div>
        <div className="text-[11px] leading-5 text-stone-500 dark:text-neutral-400">
          {WORKLOAD_MODEL_HINTS[workload.id]}
        </div>
        {resolved ? (
          <div
            className={`font-mono text-[11px] truncate ${
              isCustom ? 'text-sky-700 dark:text-sky-200' : 'text-stone-500 dark:text-neutral-400'
            }`}>
            {resolved}
          </div>
        ) : (
          <div className="text-[11px] text-stone-400 dark:text-neutral-500">
            {t('settings.ai.workload.noModel')}
          </div>
        )}
      </div>
      <button
        type="button"
        onClick={onCustomClick}
        className={`shrink-0 rounded-lg px-3 py-2 text-xs font-medium transition-colors ${
          isCustom
            ? 'bg-stone-100 text-stone-700 ring-1 ring-stone-300 dark:bg-neutral-800 dark:text-neutral-200 dark:ring-neutral-700'
            : 'bg-stone-100 text-stone-700 hover:bg-stone-200 dark:bg-neutral-800 dark:text-neutral-200 dark:hover:bg-neutral-700'
        }`}>
        {isCustom ? t('settings.ai.workload.changeModel') : t('settings.ai.workload.chooseModel')}
      </button>
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

function providerRefSignature(ref: ProviderRef): string {
  switch (ref.kind) {
    case 'openhuman':
      return 'openhuman';
    case 'default':
      return 'default';
    case 'cloud':
      return `cloud:${ref.providerSlug}:${ref.model}:${ref.temperature ?? ''}`;
    case 'local':
      return `local:${ref.model}:${ref.temperature ?? ''}`;
  }
}

function inferRoutingMode(routing: RoutingMap): RoutingMode {
  const refs = ROUTING_WORKLOAD_IDS.map(id => routing[id]);
  if (refs.every(ref => ref.kind === 'openhuman' || ref.kind === 'default')) {
    return 'managed';
  }
  const first = refs[0];
  if (
    first &&
    (first.kind === 'cloud' || first.kind === 'local') &&
    refs.every(ref => providerRefSignature(ref) === providerRefSignature(first))
  ) {
    return 'own';
  }
  return 'custom';
}

function inferSharedModelRef(routing: RoutingMap): ProviderRef | null {
  const refs = ROUTING_WORKLOAD_IDS.map(id => routing[id]);
  const first = refs[0];
  if (!first) return null;
  if (refs.every(ref => providerRefSignature(ref) === providerRefSignature(first))) {
    return first.kind === 'openhuman' ? null : first;
  }
  return (
    refs.find(ref => ref.kind === 'cloud' || ref.kind === 'local' || ref.kind === 'default') ?? null
  );
}

function routingWithAllWorkloads(next: ProviderRef): RoutingMap {
  return {
    chat: next,
    reasoning: next,
    agentic: next,
    coding: next,
    memory: next,
    heartbeat: next,
    learning: next,
    subconscious: next,
  };
}

function humanizeModelId(id: string): string {
  return id.replace(/[-_]/g, ' ').replace(/\b\w/g, c => c.toUpperCase());
}

function appendTemperatureToProviderString(provider: string, temperature: number | null): string {
  if (temperature == null || !Number.isFinite(temperature)) return provider;
  const rounded = Math.round(temperature * 100) / 100;
  return `${provider}@${String(rounded)}`;
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
  // "Custom" options. OpenHuman is its own Managed path; Default serializes
  // to the backend's `cloud` sentinel.
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
  const [testBusy, setTestBusy] = useState(false);
  const [testReply, setTestReply] = useState<string | null>(null);
  const [testError, setTestError] = useState<string | null>(null);
  const [testStartedAt, setTestStartedAt] = useState<string | null>(null);
  const testRequestIdRef = useRef(0);
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
  const canTest = canSave && !cloudModelsLoading;

  const resetTestState = () => {
    testRequestIdRef.current += 1;
    setTestReply(null);
    setTestError(null);
    setTestStartedAt(null);
    setTestBusy(false);
  };

  const currentProviderString =
    source == null
      ? null
      : source.kind === 'cloud'
        ? appendTemperatureToProviderString(
            `${source.providerSlug}:${model.trim()}`,
            temperature == null || !Number.isFinite(temperature) ? null : temperature
          )
        : appendTemperatureToProviderString(
            `ollama:${model.trim()}`,
            temperature == null || !Number.isFinite(temperature) ? null : temperature
          );

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

  const handleTest = async () => {
    if (!currentProviderString || !canTest) return;
    const requestId = testRequestIdRef.current + 1;
    testRequestIdRef.current = requestId;
    setTestBusy(true);
    setTestReply(null);
    setTestError(null);
    setTestStartedAt(new Date().toLocaleTimeString());
    try {
      const result = await testProviderModel(workload.id, currentProviderString, 'Hello world');
      if (testRequestIdRef.current !== requestId) return;
      setTestReply(result.reply);
    } catch (err) {
      if (testRequestIdRef.current !== requestId) return;
      setTestError(err instanceof Error ? err.message : String(err));
    } finally {
      if (testRequestIdRef.current === requestId) {
        setTestBusy(false);
      }
    }
  };

  const noProviders = customCloud.length === 0 && !localAvailable;

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label={formatI18n(t('settings.ai.customRoutingForWorkload'), { label: workload.label })}
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/30 p-4">
      <div className="w-full max-w-md rounded-2xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 p-6 shadow-soft">
        <div className="flex items-start justify-between gap-3 mb-4">
          <div>
            <h3 className="text-base font-semibold text-stone-900 dark:text-neutral-100">
              {t('settings.ai.customRouting')}
            </h3>
            <p className="mt-0.5 text-xs text-stone-500 dark:text-neutral-400">{workload.label}</p>
            <p className="mt-2 max-w-md text-xs leading-5 text-stone-500 dark:text-neutral-400">
              {WORKLOAD_MODEL_HINTS[workload.id]}
            </p>
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
                  resetTestState();
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
                  onChange={e => {
                    resetTestState();
                    setModel(e.target.value);
                  }}
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
                  <option>{t('settings.ai.loadingModels')}</option>
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
                      {t('common.retry')}
                    </button>
                    <span className="text-xs text-stone-400 dark:text-neutral-500">
                      {t('settings.ai.enterModelIdManually')}
                    </span>
                  </div>
                  <input
                    type="text"
                    value={model}
                    onChange={e => {
                      resetTestState();
                      setModel(e.target.value);
                    }}
                    placeholder={
                      selectedCloud
                        ? formatI18n(t('settings.ai.modelIdPlaceholderForProvider'), {
                            slug: selectedCloud.slug,
                          })
                        : t('settings.ai.modelIdPlaceholder')
                    }
                    className="w-full rounded-lg border border-stone-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-2 text-sm font-mono text-stone-900 dark:text-neutral-100 placeholder-stone-400 dark:placeholder-neutral-500 focus:border-primary-500 focus:outline-none focus:ring-1 focus:ring-primary-500"
                  />
                </div>
              ) : cloudModels.length > 0 ? (
                <select
                  value={model}
                  onChange={e => {
                    resetTestState();
                    setModel(e.target.value);
                  }}
                  className="rounded-lg border border-stone-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 focus:border-primary-500 focus:outline-none focus:ring-1 focus:ring-primary-500">
                  {!model && <option value="">{t('settings.ai.selectModel')}</option>}
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
                  onChange={e => {
                    resetTestState();
                    setModel(e.target.value);
                  }}
                  placeholder={
                    selectedCloud
                      ? formatI18n(t('settings.ai.modelIdPlaceholderForProvider'), {
                          slug: selectedCloud.slug,
                        })
                      : t('settings.ai.modelIdPlaceholder')
                  }
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
                    onChange={e => {
                      resetTestState();
                      setTemperature(e.target.checked ? 0.7 : null);
                    }}
                    className="h-3.5 w-3.5 rounded border-stone-300 dark:border-neutral-700 text-primary-500 focus:ring-primary-500"
                  />
                  {t('settings.ai.temperatureOverride')}
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
                    aria-label={t('settings.ai.temperatureOverrideSlider')}
                    min={0}
                    max={2}
                    step={0.05}
                    value={temperature}
                    onChange={e => {
                      resetTestState();
                      setTemperature(Number(e.target.value));
                    }}
                    className="flex-1 accent-primary-500"
                  />
                  <input
                    type="number"
                    aria-label={t('settings.ai.temperatureOverrideValue')}
                    min={0}
                    max={2}
                    step={0.05}
                    value={temperature}
                    onChange={e => {
                      const v = Number(e.target.value);
                      if (Number.isFinite(v)) {
                        resetTestState();
                        setTemperature(Math.max(0, Math.min(2, v)));
                      }
                    }}
                    className="w-16 rounded-lg border border-stone-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-2 py-1 text-xs font-mono text-stone-900 dark:text-neutral-100 focus:border-primary-500 focus:outline-none focus:ring-1 focus:ring-primary-500"
                  />
                </div>
              )}
              <p className="text-[11px] text-stone-400 dark:text-neutral-500">
                {t('settings.ai.temperatureOverrideDesc')}
              </p>
            </div>

            {(testBusy || testReply || testError || testStartedAt) && (
              <div
                role={testError ? 'alert' : 'status'}
                className={`rounded-lg border px-3 py-2 text-xs ${
                  testError
                    ? 'border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 text-coral-700 dark:text-coral-300'
                    : testBusy
                      ? 'border-amber-200 dark:border-amber-500/30 bg-amber-50 dark:bg-amber-500/10 text-amber-800 dark:text-amber-200'
                      : 'border-sage-200 dark:border-sage-500/30 bg-sage-50 dark:bg-sage-500/10 text-sage-800 dark:text-sage-200'
                }`}>
                <div className="font-semibold">
                  {testError
                    ? t('settings.ai.testFailed')
                    : testBusy
                      ? t('settings.ai.testingModel')
                      : t('settings.ai.modelResponse')}
                </div>
                <div className="mt-1 space-y-1">
                  <div className="font-mono text-[11px] text-current/80">
                    {formatI18n(t('settings.ai.providerWithValue'), {
                      value: currentProviderString ?? t('settings.ai.noneDash'),
                    })}
                  </div>
                  <div className="font-mono text-[11px] text-current/80">
                    {t('settings.ai.promptHelloWorld')}
                  </div>
                  {testStartedAt && (
                    <div className="font-mono text-[11px] text-current/80">
                      {formatI18n(t('settings.ai.startedAt'), { value: testStartedAt })}
                    </div>
                  )}
                </div>
                {testBusy ? (
                  <div className="mt-2 rounded-md border border-current/15 bg-white/50 px-3 py-2 text-[12px] dark:bg-black/10">
                    {t('settings.ai.waitingForModelResponse')}
                  </div>
                ) : testError ? (
                  <div className="mt-2 rounded-md border border-current/15 bg-white/50 px-3 py-2 font-mono text-[11px] whitespace-pre-wrap break-words dark:bg-black/10">
                    {testError}
                  </div>
                ) : (
                  <div className="mt-3 space-y-1.5">
                    <div className="text-[11px] font-semibold uppercase tracking-wide text-current/80">
                      {t('settings.ai.response')}
                    </div>
                    <div className="rounded-md border border-current/15 bg-white/70 px-3 py-3 text-[13px] leading-relaxed text-stone-900 whitespace-pre-wrap break-words dark:bg-black/10 dark:text-neutral-100">
                      {testReply}
                    </div>
                  </div>
                )}
              </div>
            )}
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
            onClick={() => void handleTest()}
            disabled={!canTest || testBusy}
            className="rounded-lg border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-4 py-2 text-sm font-medium text-stone-700 dark:text-neutral-200 hover:bg-stone-50 dark:hover:bg-neutral-800/60 disabled:cursor-not-allowed disabled:opacity-50">
            {testBusy ? t('settings.ai.testing') : t('settings.ai.test')}
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

const GlobalOwnModelSelector = ({
  current,
  saved,
  cloudProviders,
  localModels,
  ollamaRunning,
  onApply,
}: {
  current: ProviderRef | null;
  saved: ProviderRef | null;
  cloudProviders: CloudProvider[];
  localModels: OllamaModel[];
  ollamaRunning: boolean;
  onApply: (next: ProviderRef) => Promise<void>;
}) => {
  const { t } = useT();
  const customCloud = cloudProviders.filter(p => p.slug !== 'openhuman');
  const localAvailable = ollamaRunning && localModels.length > 0;

  const initialSource: CustomDialogSource | null =
    current?.kind === 'cloud'
      ? { kind: 'cloud', providerSlug: current.providerSlug }
      : current?.kind === 'local'
        ? { kind: 'local' }
        : customCloud[0]
          ? { kind: 'cloud', providerSlug: customCloud[0].slug }
          : localAvailable
            ? { kind: 'local' }
            : null;

  const [source, setSource] = useState<CustomDialogSource | null>(initialSource);
  const [model, setModel] = useState<string>(
    current?.kind === 'cloud' || current?.kind === 'local' ? current.model : ''
  );
  const [cloudModels, setCloudModels] = useState<ModelInfo[]>([]);
  const [cloudModelsLoading, setCloudModelsLoading] = useState(false);
  const [cloudModelsError, setCloudModelsError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const selectedSlug = source?.kind === 'cloud' ? source.providerSlug : null;

  useEffect(() => {
    if (!selectedSlug) {
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
    listProviderModels(provider.slug)
      .then(ms => {
        if (!active) return;
        setCloudModels(ms);
        setCloudModelsLoading(false);
        if (!model.trim() && ms[0]?.id) {
          setModel(ms[0].id);
        }
      })
      .catch((err: unknown) => {
        if (!active) return;
        setCloudModelsError(err instanceof Error ? err.message : String(err));
        setCloudModelsLoading(false);
      });
    return () => {
      active = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedSlug]);

  useEffect(() => {
    if (source?.kind === 'local' && !model.trim()) {
      setModel(localModels[0]?.id ?? '');
    }
  }, [source, localModels, model]);

  const canApply = source !== null && model.trim().length > 0;
  const selectedRef =
    !source || !model.trim()
      ? null
      : source.kind === 'local'
        ? ({ kind: 'local', model: model.trim() } as const)
        : ({ kind: 'cloud', providerSlug: source.providerSlug, model: model.trim() } as const);
  const isSaved =
    selectedRef !== null &&
    saved !== null &&
    providerRefSignature(selectedRef) === providerRefSignature(saved);

  const applySelection = async (nextSource: CustomDialogSource | null, nextModel: string) => {
    if (!nextSource || !nextModel.trim()) return;
    setSaving(true);
    try {
      if (nextSource.kind === 'local') {
        await onApply({ kind: 'local', model: nextModel.trim() });
      } else {
        await onApply({
          kind: 'cloud',
          providerSlug: nextSource.providerSlug,
          model: nextModel.trim(),
        });
      }
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="space-y-4 rounded-xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 p-4">
      <div className="space-y-1">
        <div className="text-sm font-medium text-stone-900 dark:text-neutral-100">
          {t('settings.ai.globalModel.title')}
        </div>
        <p className="text-xs text-amber-700 dark:text-amber-200">
          {t('settings.ai.globalModel.desc')}
        </p>
      </div>

      {customCloud.length === 0 && !localAvailable ? (
        <div className="rounded-lg border border-amber-200 dark:border-amber-500/30 bg-amber-50 dark:bg-amber-500/10 p-3 text-xs text-amber-800 dark:text-amber-200">
          {t('settings.ai.globalModel.noProviders')}
        </div>
      ) : (
        <>
          <div className="grid gap-4 md:grid-cols-2">
            <div className="space-y-1.5">
              <label className="text-xs font-medium text-stone-700 dark:text-neutral-200">
                {t('settings.ai.globalModel.provider')}
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
                    const nextSource = { kind: 'local' } as const;
                    const nextModel = localModels[0]?.id ?? '';
                    setSource(nextSource);
                    setModel(nextModel);
                  } else {
                    const nextSource = { kind: 'cloud', providerSlug: slug } as const;
                    setSource(nextSource);
                    setModel('');
                  }
                }}
                className="w-full rounded-lg border border-stone-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100">
                {customCloud.map(p => (
                  <option key={p.slug} value={`cloud:${p.slug}`}>
                    {p.label}
                  </option>
                ))}
                {localAvailable ? (
                  <option value="local:">{t('settings.ai.provider.ollama')}</option>
                ) : null}
              </select>
            </div>

            <div className="space-y-1.5">
              <label className="text-xs font-medium text-stone-700 dark:text-neutral-200">
                {t('settings.ai.globalModel.model')}
              </label>
              {source?.kind === 'local' ? (
                <select
                  value={model}
                  onChange={e => setModel(e.target.value)}
                  className="w-full rounded-lg border border-stone-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100">
                  {localModels.map(m => (
                    <option key={m.id} value={m.id}>
                      {m.id}
                    </option>
                  ))}
                </select>
              ) : cloudModels.length > 0 ? (
                <select
                  value={model}
                  onChange={e => setModel(e.target.value)}
                  className="w-full rounded-lg border border-stone-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100">
                  {cloudModels.map(m => (
                    <option key={m.id} value={m.id}>
                      {m.id}
                    </option>
                  ))}
                </select>
              ) : (
                <input
                  value={model}
                  onChange={e => setModel(e.target.value)}
                  placeholder={
                    cloudModelsLoading
                      ? t('settings.ai.globalModel.loadingModels')
                      : t('settings.ai.globalModel.enterModelId')
                  }
                  className="w-full rounded-lg border border-stone-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100"
                />
              )}
              {cloudModelsError ? (
                <div className="text-xs text-coral-700 dark:text-coral-300">{cloudModelsError}</div>
              ) : null}
            </div>
          </div>
          <div className="rounded-lg bg-stone-50 dark:bg-neutral-800/60 px-3 py-2 text-xs text-stone-500 dark:text-neutral-400">
            {t('settings.ai.globalModel.appliesToAll')}
          </div>

          <div className="flex justify-end">
            <button
              type="button"
              disabled={!canApply || saving || isSaved}
              onClick={() => void applySelection(source, model)}
              className="rounded-lg bg-primary-500 px-3 py-2 text-xs font-medium text-white hover:bg-primary-600 disabled:cursor-not-allowed disabled:opacity-50">
              {saving
                ? t('settings.ai.globalModel.saving')
                : isSaved
                  ? t('settings.ai.globalModel.saved')
                  : t('common.save')}
            </button>
          </div>
        </>
      )}
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
  const { saved, draft, isDirty, save, persist, discard, loading, error, reload } = useAISettings();
  // #1574 §4b: advisory re-embed modal, driven by the backend status RPC.
  // Logic lives in a unit-testable hook (see useReembedBackfillModal).
  const { reembed, handleSave, dismissReembed } = useReembedBackfillModal(save);
  const ollama = useOllamaStatus();
  const installed = useInstalledModels(ollama.snapshot);
  const [editing, setEditing] = useState<CloudProvider | 'new' | null>(null);
  const [busyAction, setBusyAction] = useState<string | null>(null);
  const [codexAuthError, setCodexAuthError] = useState<string | null>(null);
  // Which workload's "Custom" dialog is currently open (null = closed).
  const [customDialogFor, setCustomDialogFor] = useState<WorkloadId | null>(null);
  const [routingEditorMode, setRoutingEditorMode] = useState<'own' | 'custom' | null>(null);
  // Which provider slug's API-key dialog is currently open (null = closed).
  const [keyDialogFor, setKeyDialogFor] = useState<string | null>(null);
  // When the user toggles LM Studio / Ollama (local runtimes), we
  // need to remember which label to attach to the upserted provider so the
  // chip can find it again. Cleared when the dialog closes.
  const [pendingLocalLabel, setPendingLocalLabel] = useState<string | null>(null);
  const openRouterOauthAbortRef = useRef<AbortController | null>(null);

  const connectProvider = useCallback(
    async ({
      slug,
      localLabel = null,
      value,
      credentialMode,
    }: {
      slug: string;
      localLabel?: string | null;
      value: string;
      credentialMode: 'api_key' | 'oauth' | 'codex_oauth' | 'endpoint';
    }) => {
      const isLocalRuntime = credentialMode === 'endpoint';
      const isCodexOAuth = credentialMode === 'codex_oauth';
      setBusyAction(`toggle-${localLabel ? localLabel.toLowerCase().replace(/\s/g, '') : slug}`);

      try {
        const trimmed = value.trim();
        const endpoint = isLocalRuntime
          ? (() => {
              const url = new URL(trimmed);
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

        const priorWireProviders = saved.cloudProviders.map(p => ({
          id: p.id,
          slug: p.slug,
          label: p.label,
          endpoint: p.endpoint,
          auth_style: p.authStyle,
        }));

        if (!isLocalRuntime && !isCodexOAuth && slug !== 'openhuman') {
          await setCloudProviderKey(slug, trimmed);
        } else if (isLocalRuntime && slug === 'ollama') {
          const baseUrl = endpoint.replace(/\/v1\/?$/, '');
          await openhumanUpdateLocalAiSettings({
            base_url: baseUrl,
            provider: 'ollama',
            runtime_enabled: true,
            opt_in_confirmed: true,
          });
        } else if (isLocalRuntime && slug === 'lmstudio') {
          await openhumanUpdateLocalAiSettings({
            base_url: endpoint,
            provider: 'lm_studio',
            runtime_enabled: true,
            opt_in_confirmed: true,
          });
        }

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
          if (!isCodexOAuth) {
            try {
              await listProviderModels(slug);
            } catch (probeErr) {
              await flushCloudProviders(priorWireProviders).catch(() => {});
              if (!isLocalRuntime && slug !== 'openhuman') {
                await clearCloudProviderKey(slug).catch(() => {});
              }
              const msg = probeErr instanceof Error ? probeErr.message : String(probeErr);
              throw new Error(`Could not reach ${upserted.label}: ${msg}`);
            }
          }
        }

        const nextDraft = {
          ...draft,
          cloudProviders: [...draft.cloudProviders.filter(p => p.slug !== slug), upserted],
        };
        await persist(nextDraft);
        if (isCodexOAuth && slug === 'openai') {
          await clearCloudProviderKey(slug);
        }
        if (slug === 'openai') {
          setCodexAuthError(null);
        }
        setKeyDialogFor(null);
        setPendingLocalLabel(null);
      } finally {
        setBusyAction(null);
      }
    },
    [draft, persist, saved.cloudProviders]
  );

  const connectOpenAiViaCodexAuth = useCallback(async () => {
    setCodexAuthError(null);
    setBusyAction('codex-auth');
    try {
      await importOpenAiCodexCliAuth();
      await connectProvider({ slug: 'openai', value: 'oauth', credentialMode: 'codex_oauth' });
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      const localizedMessage =
        message === OPENAI_CODEX_OAUTH_MISSING_AUTH_URL
          ? t('settings.ai.codexOauthMissingAuthUrl')
          : message === OPENAI_CODEX_OAUTH_MISSING_CALLBACK_URL
            ? t('settings.ai.codexOauthMissingCallbackUrl')
            : message;
      console.warn('[ai-settings] codex auth import failed', {
        summary: presentProviderSetupError(localizedMessage, t).summary,
      });
      setCodexAuthError(localizedMessage);
    } finally {
      setBusyAction(null);
    }
  }, [connectProvider, t]);

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
          if (r.kind === 'default') return 'cloud';
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
  const inferredRoutingMode = useMemo(() => inferRoutingMode(draft.routing), [draft.routing]);
  const effectiveRoutingMode: RoutingMode =
    routingEditorMode === 'own'
      ? 'own'
      : routingEditorMode === 'custom'
        ? 'custom'
        : inferredRoutingMode;
  const sharedModelRef = useMemo(() => inferSharedModelRef(draft.routing), [draft.routing]);

  return (
    <div className="relative">
      {!embedded && (
        <SettingsHeader
          title={t('pages.settings.ai.llm')}
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
              <ProviderToggleChip
                key="openhuman"
                slug="openhuman"
                label={t('settings.ai.routing.managed')}
                enabled
                locked
                onToggle={() => {}}
              />

              {/* Built-in cloud providers — openai/anthropic/openrouter/orcarouter/custom */}
              {(
                [
                  'openai',
                  'anthropic',
                  'openrouter',
                  'orcarouter',
                  'gmi',
                  'fireworks',
                  'moonshot',
                ] as const
              ).map(slug => {
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
                    onToggle={async () => {
                      if (enabled && existing) {
                        // Toggle OFF: remove the provider + scrub any
                        // routing entries that pin to it.
                        const remaining = draft.cloudProviders.filter(cp => cp.id !== existing.id);
                        const nextRouting = Object.fromEntries(
                          Object.entries(draft.routing).map(([wid, ref]) => [
                            wid,
                            ref.kind === 'cloud' && ref.providerSlug === existing.slug
                              ? ({ kind: 'default' } as const)
                              : ref,
                          ])
                        ) as typeof draft.routing;
                        await persist({
                          ...draft,
                          cloudProviders: remaining,
                          routing: nextRouting,
                        });
                      } else {
                        // Toggle ON: open the API-key popup. The chip
                        // only flips after the dialog saves.
                        setKeyDialogFor(slug);
                      }
                    }}
                  />
                );
              })}

              {draft.cloudProviders
                .filter(
                  cp =>
                    ![
                      'openhuman',
                      'openai',
                      'anthropic',
                      'openrouter',
                      'orcarouter',
                      'gmi',
                      'fireworks',
                      'moonshot',
                      'lmstudio',
                      'ollama',
                    ].includes(cp.slug)
                )
                .map(existing => (
                  <ProviderToggleChip
                    key={existing.id}
                    slug="custom"
                    label={existing.label}
                    enabled
                    busy={busyAction === `toggle-${existing.slug}`}
                    onToggle={async () => {
                      const remaining = draft.cloudProviders.filter(cp => cp.id !== existing.id);
                      const nextRouting = Object.fromEntries(
                        Object.entries(draft.routing).map(([wid, ref]) => [
                          wid,
                          ref.kind === 'cloud' && ref.providerSlug === existing.slug
                            ? ({ kind: 'default' } as const)
                            : ref,
                        ])
                      ) as typeof draft.routing;
                      await persist({ ...draft, cloudProviders: remaining, routing: nextRouting });
                    }}
                  />
                ))}

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
                      aria-label={providerToggleAriaLabel(t, enabled, label)}
                      disabled={busyAction === `toggle-${localKind}`}
                      onClick={async () => {
                        if (enabled && existing) {
                          const remaining = draft.cloudProviders.filter(
                            cp => cp.id !== existing.id
                          );
                          const nextRouting = Object.fromEntries(
                            Object.entries(draft.routing).map(([wid, ref]) => [
                              wid,
                              ref.kind === 'cloud' && ref.providerSlug === localKind
                                ? ({ kind: 'default' } as const)
                                : ref,
                            ])
                          ) as typeof draft.routing;
                          await persist({
                            ...draft,
                            cloudProviders: remaining,
                            routing: nextRouting,
                          });
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

            <div className="flex flex-col gap-2 pt-1">
              <div className="flex flex-wrap items-center gap-2">
                <button
                  type="button"
                  onClick={() => void connectOpenAiViaCodexAuth()}
                  disabled={busyAction === 'codex-auth' || busyAction === 'toggle-openai'}
                  className="inline-flex items-center gap-2 rounded-lg border border-stone-200 bg-white px-3 py-2 text-xs font-medium text-stone-900 transition-colors hover:bg-stone-50 disabled:cursor-wait disabled:opacity-60 dark:border-neutral-800 dark:bg-neutral-900 dark:text-neutral-100 dark:hover:bg-neutral-800">
                  <LuKeyRound className="h-3.5 w-3.5" />
                  {busyAction === 'codex-auth' || busyAction === 'toggle-openai'
                    ? t('settings.ai.connecting')
                    : t('settings.ai.codexAuthButton', 'Codex 인증')}
                </button>
                <span className="text-xs text-stone-500 dark:text-neutral-400">
                  {t(
                    'settings.ai.codexAuthHelper',
                    'Uses the existing Codex CLI login from ~/.codex/auth.json.'
                  )}
                </span>
              </div>
              {codexAuthError ? <ProviderSetupErrorNotice error={codexAuthError} /> : null}
            </div>

            <div className="pt-1">
              <button
                type="button"
                onClick={() => setEditing('new')}
                className="inline-flex items-center gap-2 rounded-lg bg-primary-50 px-3 py-2 text-xs font-medium text-primary-900 ring-1 ring-primary-200 transition-colors hover:bg-primary-100 dark:bg-primary-500/10 dark:text-primary-100 dark:ring-primary-500/30 dark:hover:bg-primary-500/20">
                {t('settings.ai.routing.addCustomProvider')}
              </button>
            </div>
          </section>
        </div>
        {/* end of Auth section */}

        {/* ═══════════════════════════════════════════════════════════════
            ROUTING — top-level routing mode. Managed = OpenHuman decides.
            Own = one provider/model for everything. Custom = fine-grained
            per-workload routing.
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
            <div className="grid gap-3 md:grid-cols-3">
              <button
                type="button"
                onClick={async () => {
                  setRoutingEditorMode(null);
                  await persist({
                    ...draft,
                    routing: routingWithAllWorkloads({ kind: 'openhuman' }),
                  });
                }}
                className={`flex h-full min-h-[152px] flex-col rounded-2xl border p-4 text-left transition-colors ${
                  effectiveRoutingMode === 'managed'
                    ? 'border-emerald-300 bg-emerald-50 dark:border-emerald-500/40 dark:bg-emerald-500/10'
                    : 'border-stone-200 bg-white hover:bg-stone-50 dark:border-neutral-800 dark:bg-neutral-900 dark:hover:bg-neutral-800'
                }`}>
                <div className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
                  {t('settings.ai.routing.managed')}
                </div>
                <p className="mt-2 text-xs leading-5 text-stone-600 dark:text-neutral-300">
                  {t('settings.ai.routing.managedDesc')}
                </p>
              </button>

              <button
                type="button"
                onClick={() => setRoutingEditorMode('own')}
                className={`flex h-full min-h-[152px] flex-col rounded-2xl border p-4 text-left transition-colors ${
                  effectiveRoutingMode === 'own'
                    ? 'border-sky-300 bg-sky-50 dark:border-sky-500/40 dark:bg-sky-500/10'
                    : 'border-stone-200 bg-white hover:bg-stone-50 dark:border-neutral-800 dark:bg-neutral-900 dark:hover:bg-neutral-800'
                }`}>
                <div className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
                  {t('settings.ai.routing.useYourOwn')}
                </div>
                <p className="mt-2 text-xs leading-5 text-stone-600 dark:text-neutral-300">
                  {t('settings.ai.routing.useYourOwnDesc')}
                </p>
              </button>

              <button
                type="button"
                onClick={() => setRoutingEditorMode('custom')}
                className={`flex h-full min-h-[152px] flex-col rounded-2xl border p-4 text-left transition-colors ${
                  effectiveRoutingMode === 'custom'
                    ? 'border-sky-300 bg-sky-50 dark:border-sky-500/40 dark:bg-sky-500/10'
                    : 'border-stone-200 bg-white hover:bg-stone-50 dark:border-neutral-800 dark:bg-neutral-900 dark:hover:bg-neutral-800'
                }`}>
                <div className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
                  {t('settings.ai.routing.advanced')}
                </div>
                <p className="mt-2 text-xs leading-5 text-stone-600 dark:text-neutral-300">
                  {t('settings.ai.routing.advancedDesc')}
                </p>
              </button>
            </div>

            {effectiveRoutingMode === 'managed' ? (
              <div className="rounded-xl border border-emerald-200 bg-emerald-50/70 px-4 py-3 text-sm text-emerald-900 dark:border-emerald-500/30 dark:bg-emerald-500/10 dark:text-emerald-100">
                {t('settings.ai.routing.managedMsg')}
              </div>
            ) : null}

            {effectiveRoutingMode === 'own' ? (
              <GlobalOwnModelSelector
                current={sharedModelRef}
                saved={inferSharedModelRef(saved.routing)}
                cloudProviders={draft.cloudProviders}
                localModels={installed}
                ollamaRunning={ollama.state === 'running'}
                onApply={async next => {
                  await persist({ ...draft, routing: routingWithAllWorkloads(next) });
                }}
              />
            ) : null}

            {effectiveRoutingMode === 'custom' ? (
              <>
                <div className="rounded-xl border border-sky-200 bg-sky-50/70 px-4 py-3 text-sm text-sky-900 dark:border-sky-500/30 dark:bg-sky-500/10 dark:text-sky-100">
                  {t('settings.ai.routing.customDesc')}
                </div>

                <div className="space-y-3">
                  <div className="overflow-hidden rounded-lg border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 px-3">
                    <div className="border-b border-stone-200 dark:border-neutral-800 py-3">
                      <div className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
                        {t('settings.ai.routing.chatAndConversations')}
                      </div>
                      <div className="mt-1 text-xs text-stone-500 dark:text-neutral-400">
                        {t('settings.ai.routing.chatDesc')}
                      </div>
                    </div>
                    <div className="divide-y divide-stone-200 dark:divide-neutral-800">
                      {chatRows.map(w => (
                        <WorkloadRow
                          key={w.id}
                          workload={w}
                          ref_={draft.routing[w.id]}
                          cloudProviders={draft.cloudProviders}
                          onCustomClick={() => setCustomDialogFor(w.id)}
                        />
                      ))}
                    </div>
                  </div>

                  <div className="overflow-hidden rounded-lg border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 px-3">
                    <div className="border-b border-stone-200 dark:border-neutral-800 py-3">
                      <div className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
                        {t('settings.ai.routing.backgroundTasks')}
                      </div>
                      <div className="mt-1 text-xs text-stone-500 dark:text-neutral-400">
                        {t('settings.ai.routing.bgTasksDesc')}
                      </div>
                    </div>
                    <div className="divide-y divide-stone-200 dark:divide-neutral-800">
                      {bgRows.map(w => (
                        <WorkloadRow
                          key={w.id}
                          workload={w}
                          ref_={draft.routing[w.id]}
                          cloudProviders={draft.cloudProviders}
                          onCustomClick={() => setCustomDialogFor(w.id)}
                        />
                      ))}
                    </div>
                  </div>
                </div>
              </>
            ) : null}
          </section>
        </div>
        {/* end of Routing section */}
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
          title: t('settings.ai.reindexingMemory'),
          message: formatI18n(t('settings.ai.reindexingMemoryMessage'), {
            pending: reembed.pending,
          }),
          confirmText: t('common.ok'),
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
              await persist({ ...draft, cloudProviders: list });
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
          oauthAction={
            keyDialogFor === 'openrouter' && !pendingLocalLabel
              ? {
                  label: t('settings.ai.signInWithOpenRouter'),
                  onClick: async () => {
                    const controller = new AbortController();
                    openRouterOauthAbortRef.current = controller;
                    try {
                      const apiKey = await connectOpenRouterViaOAuth({ signal: controller.signal });
                      await connectProvider({
                        slug: 'openrouter',
                        value: apiKey,
                        credentialMode: 'oauth',
                      });
                    } finally {
                      if (openRouterOauthAbortRef.current === controller) {
                        openRouterOauthAbortRef.current = null;
                      }
                    }
                  },
                }
              : null
          }
          onCancel={() => {
            openRouterOauthAbortRef.current?.abort();
            openRouterOauthAbortRef.current = null;
            setKeyDialogFor(null);
            setPendingLocalLabel(null);
          }}
          onSubmit={async value =>
            await connectProvider({
              slug: keyDialogFor,
              localLabel: pendingLocalLabel,
              value,
              credentialMode: pendingLocalLabel ? 'endpoint' : 'api_key',
            })
          }
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
  const [label, setLabel] = useState<string>(initial?.label ?? '');
  const [endpoint, setEndpoint] = useState(initial?.endpoint ?? '');
  const [apiKey, setApiKey] = useState('');
  const [saving, setSaving] = useState(false);
  const [submitError, setSubmitError] = useState<string | null>(null);
  const slug = initial?.slug ?? slugifyCustomProviderName(label);
  const hasReservedSlugCollision =
    !initial &&
    [
      'cloud',
      'openhuman',
      'pid',
      'openai',
      'anthropic',
      'openrouter',
      'orcarouter',
      'gmi',
      'fireworks',
      'moonshot',
      'custom',
      'ollama',
      'lmstudio',
    ].includes(slug);
  const slugError = !slug
    ? t('settings.ai.slugMissingError')
    : existingSlugs.includes(slug)
      ? t('settings.ai.slugInUseError')
      : hasReservedSlugCollision
        ? t('settings.ai.slugReservedError')
        : null;
  const hasExistingKey = (initial?.maskedKey ?? '').startsWith('••••');

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-stone-900/30 p-4">
      <div className="w-full max-w-md rounded-lg border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 shadow-float">
        <div className="border-b border-stone-200 dark:border-neutral-800 px-4 py-3">
          <div className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
            {initial
              ? formatI18n(t('settings.ai.editProvider'), { label: initial.label })
              : t('settings.ai.addCloudProvider')}
          </div>
          <div className="mt-0.5 text-xs text-stone-500 dark:text-neutral-400">
            {t('settings.ai.apiKeysEncrypted')}{' '}
            <span className="font-mono">auth-profiles.json</span>.
          </div>
        </div>
        <div className="space-y-3 px-4 py-3">
          <div>
            <label
              htmlFor="cloud-provider-name"
              className="text-[10px] font-semibold uppercase tracking-wide text-stone-500 dark:text-neutral-400">
              {t('common.name')}
            </label>
            <input
              id="cloud-provider-name"
              value={label}
              onChange={e => setLabel(e.target.value)}
              className="mt-1 w-full rounded-lg border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 placeholder:text-stone-400 dark:placeholder:text-neutral-500 dark:text-neutral-500 dark:placeholder:text-neutral-500 focus:border-primary-400 focus:outline-none focus:ring-1 focus:ring-primary-200"
              placeholder={t('settings.ai.providerNamePlaceholder')}
            />
            <div className="mt-1 text-[11px] text-stone-500 dark:text-neutral-400">
              {t('settings.ai.slugLabel')}{' '}
              <span className="font-mono text-stone-700 dark:text-neutral-200">
                {slug || t('settings.ai.noneDash')}
              </span>
            </div>
            {slugError ? (
              <div className="mt-1 text-[11px] text-coral-600 dark:text-coral-300">{slugError}</div>
            ) : null}
          </div>
          <div>
            <label
              htmlFor="cloud-provider-openai-url"
              className="text-[10px] font-semibold uppercase tracking-wide text-stone-500 dark:text-neutral-400">
              {t('settings.ai.openAiUrlLabel')}
            </label>
            <input
              id="cloud-provider-openai-url"
              value={endpoint}
              onChange={e => setEndpoint(e.target.value)}
              className="mt-1 w-full rounded-lg border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-3 py-2 font-mono text-xs text-stone-900 dark:text-neutral-100 placeholder:text-stone-400 dark:placeholder:text-neutral-500 dark:text-neutral-500 dark:placeholder:text-neutral-500 disabled:opacity-60 focus:border-primary-400 focus:outline-none focus:ring-1 focus:ring-primary-200"
              placeholder={t('settings.ai.openAiUrlPlaceholder')}
            />
          </div>
          <div>
            <label className="flex items-center justify-between text-[10px] font-semibold uppercase tracking-wide text-stone-500 dark:text-neutral-400">
              <span>{t('settings.ai.apiKeyFieldLabel')}</span>
              {hasExistingKey && (
                <button
                  onClick={() => void onClearKey(slug)}
                  className="text-[10px] font-medium normal-case text-coral-600 dark:text-coral-300 hover:text-coral-700 dark:text-coral-300">
                  {t('settings.ai.clearStoredKey')}
                </button>
              )}
            </label>
            <input
              aria-label={t('settings.ai.apiKeyFieldLabel')}
              type="text"
              autoComplete="off"
              autoCorrect="off"
              autoCapitalize="off"
              spellCheck={false}
              data-form-type="other"
              data-lpignore="true"
              data-1p-ignore="true"
              value={apiKey}
              onChange={e => setApiKey(e.target.value)}
              className="mt-1 w-full rounded-lg border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-3 py-2 font-mono text-xs text-stone-900 dark:text-neutral-100 placeholder:text-stone-400 dark:placeholder:text-neutral-500 dark:text-neutral-500 dark:placeholder:text-neutral-500 focus:border-primary-400 focus:outline-none focus:ring-1 focus:ring-primary-200"
              placeholder={hasExistingKey ? t('settings.ai.keepExistingKeyPlaceholder') : 'sk-...'}
            />
          </div>
          {submitError ? <ProviderSetupErrorNotice error={submitError} /> : null}
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
                if (slugError) {
                  throw new Error(slugError);
                }
                await onSubmit(
                  {
                    id: initial?.id ?? '',
                    slug,
                    label: label.trim() || slug,
                    endpoint: endpoint.trim(),
                    authStyle: initial?.authStyle ?? 'bearer',
                    maskedKey: maskKeyLabel(hasExistingKey || apiKey.length > 0),
                  },
                  apiKey.trim()
                );
              } catch (err) {
                // Caller throws when the live /models probe rejects — surface
                // the failure inline and keep the dialog open so the user can
                // fix the key/URL and retry.
                const message = err instanceof Error ? err.message : String(err);
                console.warn('[ai-settings] cloud provider editor submit failed', {
                  slug,
                  summary: presentProviderSetupError(message, t).summary,
                });
                setSubmitError(message);
              } finally {
                setSaving(false);
              }
            }}
            disabled={saving || !endpoint.trim() || Boolean(slugError)}
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
    case 'orcarouter':
      return 'https://api.orcarouter.ai/v1';
    case 'gmi':
      return 'https://api.gmi-serving.com/v1';
    case 'fireworks':
      return 'https://api.fireworks.ai/inference/v1';
    case 'moonshot':
      return 'https://api.moonshot.ai/v1';
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
