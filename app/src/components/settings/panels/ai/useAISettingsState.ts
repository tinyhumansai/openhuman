/*
 * API-adapter hooks for the AI settings panel.
 *
 * The panel works in terms of `CloudProvider` (slug + maskedKey) and
 * `ProviderRef` (slug-keyed). The wire format is identical — this layer
 * just derives the `maskedKey` display string from `has_api_key`.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import {
  type AISettings as ApiAISettings,
  type ProviderRef as ApiProviderRef,
  type CloudProviderView,
  flushCloudProviders,
  listProviderModels,
  loadAISettings,
  loadLocalProviderSnapshot,
  type LocalProviderSnapshot,
  saveAISettings,
} from '../../../../services/api/aiSettingsApi';
import { toSelectableChatModels } from '../aiRouting';
import {
  type AISettings,
  type CloudProvider,
  EMPTY_SETTINGS,
  maskKeyLabel,
  type OllamaModel,
  type OllamaState,
  type ProviderRef,
  type RoutingMap,
} from './aiPanelTypes';

/** Providers with no `/models` endpoint, never re-probed at save time. */
const SAVE_PROBE_EXEMPT_SLUGS = new Set(['openhuman', 'claude-code']);

function toPanelProvider(p: CloudProviderView): CloudProvider {
  return {
    id: p.id,
    slug: p.slug,
    label: p.label,
    endpoint: p.endpoint,
    caCertPem: p.ca_cert_pem ?? '',
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
    vision: liftRef(api.routing.vision),
    memory: liftRef(api.routing.memory),
  };
  return {
    panel: {
      cloudProviders,
      routing,
      modelRegistry: api.modelRegistry,
      defaultModel: api.defaultModel ?? '',
    },
  };
}

function toApiSettings(panel: AISettings): ApiAISettings {
  return {
    cloudProviders: panel.cloudProviders.map(p => ({
      id: p.id,
      slug: p.slug,
      label: p.label,
      endpoint: p.endpoint,
      ca_cert_pem: p.caCertPem ?? '',
      auth_style: p.authStyle,
      has_api_key: p.maskedKey.startsWith('••••'),
    })),
    routing: {
      chat: panel.routing.chat,
      reasoning: panel.routing.reasoning,
      agentic: panel.routing.agentic,
      coding: panel.routing.coding,
      vision: panel.routing.vision,
      memory: panel.routing.memory,
    },
    modelRegistry: panel.modelRegistry,
    defaultModel: panel.defaultModel ?? '',
  };
}

export function useAISettings() {
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
      ca_cert_pem: p.caCertPem ?? '',
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
      // Exempt: OpenHuman (session JWT) and CLI-login providers such as
      // `claude-code` (`cli://` endpoint) — neither has a /models endpoint to
      // hit, matching the add-time `isCliLogin` skip in `useProviderConnect`.
      const savedById = new Map(saved.cloudProviders.map(p => [p.id, p]));
      const toProbe = draft.cloudProviders.filter(p => {
        if (SAVE_PROBE_EXEMPT_SLUGS.has(p.slug)) return false;
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

/**
 * Translate a raw `LocalProviderSnapshot` into the `OllamaState` the panel
 * UI expects. Extracted as a pure function so it can be unit-tested without
 * rendering the hook.
 *
 * The local endpoint is user-run (Ollama / LM Studio / OpenAI-compatible); the
 * app never installs, starts or downloads anything, so there is no
 * starting / downloading / installing state to surface.
 *
 * Priority order:
 *  1. `disabled`    — config master switch is off.
 *  2. `degraded`    — server alive but slow (ollama_status === 'degraded').
 *  3. `running`     — normal healthy state (ollama_running true, not degraded).
 *  4. `unreachable` — the core could not reach the configured endpoint
 *                     (status.state === 'unreachable'); treated as offline.
 *  5. `degraded`    — the core reports the runtime degraded without a
 *                     diagnostics verdict.
 *  6. `stopped`     — catch-all / no snapshot.
 */
export function deriveOllamaState(snapshot: LocalProviderSnapshot | null): OllamaState {
  if (!snapshot) return 'stopped';
  const stateStr = snapshot.status?.state ?? '';
  if (stateStr === 'disabled') return 'disabled';
  if (snapshot.diagnostics?.ollama_status === 'degraded') return 'degraded';
  if (snapshot.diagnostics?.ollama_running) return 'running';
  if (stateStr === 'unreachable') return 'unreachable';
  if (stateStr === 'degraded') return 'degraded';
  return 'stopped';
}

export function useOllamaStatus() {
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

  const state: OllamaState = useMemo(() => deriveOllamaState(snapshot), [snapshot]);

  const version = snapshot?.diagnostics?.ollama_binary_path
    ? // Diagnostics doesn't surface a version string today; show the binary path tail.
      (snapshot.diagnostics.ollama_binary_path.split(/[\\/]/).pop() ?? '')
    : '';

  return { state, version, snapshot, refresh };
}

export function useInstalledModels(snapshot: LocalProviderSnapshot | null): OllamaModel[] {
  // Hide embedding-only models (e.g. `bge-m3`) from every LLM/chat workload
  // picker — the consumer of this hook (CustomRoutingDialog) routes a chat
  // model, never the embedder (which is
  // configured separately in EmbeddingsPanel). Selecting an embedding model as
  // chat 400s every turn on Ollama (TAURI-RUST-4P6). Filter + map live in the
  // pure, unit-tested `toSelectableChatModels` helper.
  return useMemo(() => toSelectableChatModels(snapshot?.installedModels ?? []), [snapshot]);
}
