import debug from 'debug';
import { useCallback, useEffect, useMemo, useState } from 'react';

import {
  type ClientConfig,
  type ModelRoute,
  openhumanGetClientConfig,
  openhumanUpdateModelSettings,
} from '../../../utils/tauriCommands';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const log = debug('settings:llm-provider');

const KEY_PLACEHOLDER = '••••••••••••••••';

/**
 * Task-hint slots the core router understands (see
 * `src/openhuman/providers/router.rs`). When the user picks a non-OpenHuman
 * preset we persist a `model_routes` entry for each role so the router has a
 * sensible per-task default for the chosen provider.
 */
const ROLE_HINTS = ['reasoning', 'agentic', 'coding', 'summarization'] as const;
type RoleHint = (typeof ROLE_HINTS)[number];

const ROLE_LABELS: Record<RoleHint, { label: string; help: string }> = {
  reasoning: { label: 'Reasoning', help: 'Deep, multi-step thinking and planning.' },
  agentic: { label: 'Agentic', help: 'Tool use, sub-agent delegation, function calling.' },
  coding: { label: 'Coding', help: 'Code generation, refactoring, and review.' },
  summarization: { label: 'Summarization', help: 'Fast, cheap summaries and short responses.' },
};

type RoleModels = Record<RoleHint, string>;

const EMPTY_ROLE_MODELS: RoleModels = { reasoning: '', agentic: '', coding: '', summarization: '' };

interface ProviderPreset {
  /** Stable identifier — also drives the preset card key. */
  id: string;
  label: string;
  /** OpenAI-compatible base URL ending in `/v1`. Empty = use OpenHuman default. */
  apiUrl: string;
  /** Suggested single default model id; ignored for OpenHuman (router-managed). */
  suggestedModel: string;
  /**
   * Per-role suggested models for the core router. `null` for OpenHuman since
   * its built-in router picks per task without an external model_routes table.
   */
  roleModels: RoleModels | null;
  /** Short hint shown beneath the preset row when this preset is active. */
  note: string;
  /**
   * Tailwind classes giving the card a subtle brand-aligned tint. A colour
   * cue, not a brand reproduction.
   */
  tint: { idle: string; selected: string; dot: string };
}

/**
 * Curated list of OpenAI-compatible providers (#1342). The core uses
 * `OpenAiCompatibleProvider` (`/chat/completions` shape); Anthropic ships an
 * OpenAI-compat shim at `https://api.anthropic.com/v1` so it lives here too.
 */
const PROVIDER_PRESETS: ProviderPreset[] = [
  {
    id: 'openhuman',
    label: 'OpenHuman',
    apiUrl: 'https://api.tinyhumans.ai/openai/v1/chat/completions',
    suggestedModel: '',
    roleModels: null,
    note: 'Hosted OpenHuman backend — uses your signed-in session, no API key required.',
    tint: {
      idle: 'border-stone-200 hover:border-primary-300 hover:bg-primary-50/40',
      selected: 'border-primary-500 bg-primary-100 ring-2 ring-primary-300 text-primary-900',
      dot: 'bg-primary-500',
    },
  },
  {
    id: 'openai',
    label: 'OpenAI',
    apiUrl: 'https://api.openai.com/v1/chat/completions',
    suggestedModel: 'gpt-5.5-2026-04-23',
    roleModels: {
      reasoning: 'gpt-5.5-2026-04-23',
      agentic: 'gpt-5.5-2026-04-23',
      coding: 'gpt-4o',
      summarization: 'gpt-4o-mini',
    },
    note: 'Use a key from platform.openai.com. Defaults pick gpt-5.5 for reasoning and agentic, gpt-4o for coding, gpt-4o-mini for summarization.',
    tint: {
      idle: 'border-stone-200 hover:border-sage-400 hover:bg-sage-50/40',
      selected: 'border-sage-600 bg-sage-100 ring-2 ring-sage-300 text-sage-900',
      dot: 'bg-sage-600',
    },
  },
  {
    id: 'anthropic',
    label: 'Anthropic',
    // Anthropic ships an OpenAI-compatibility shim at /v1/chat/completions
    // that maps to the same Claude models — see docs.anthropic.com/en/api/openai-sdk.
    apiUrl: 'https://api.anthropic.com/v1/chat/completions',
    suggestedModel: 'claude-sonnet-4-6',
    roleModels: {
      reasoning: 'claude-opus-4-7',
      agentic: 'claude-sonnet-4-6',
      coding: 'claude-sonnet-4-6',
      summarization: 'claude-haiku-4-5-20251001',
    },
    note: 'Uses Anthropic’s OpenAI-compatibility endpoint with a key from console.anthropic.com. Defaults: Opus 4.7 reasoning, Sonnet 4.6 agentic/coding, Haiku 4.5 summarization.',
    tint: {
      idle: 'border-stone-200 hover:border-coral-400 hover:bg-coral-50/40',
      selected: 'border-coral-600 bg-coral-100 ring-2 ring-coral-300 text-coral-900',
      dot: 'bg-coral-600',
    },
  },
  {
    id: 'openrouter',
    label: 'OpenRouter',
    apiUrl: 'https://openrouter.ai/api/v1/chat/completions',
    suggestedModel: 'openai/gpt-4o',
    roleModels: {
      reasoning: 'openai/o1',
      agentic: 'anthropic/claude-sonnet-4.6',
      coding: 'anthropic/claude-sonnet-4.6',
      summarization: 'openai/gpt-4o-mini',
    },
    note: 'One key, dozens of providers (openrouter.ai). Mix and match per role — swap to meta-llama/llama-3.3-70b-instruct, google/gemini-2.0-flash, etc.',
    tint: {
      idle: 'border-stone-200 hover:border-amber-400 hover:bg-amber-50/40',
      selected: 'border-amber-600 bg-amber-100 ring-2 ring-amber-300 text-amber-900',
      dot: 'bg-amber-500',
    },
  },
  {
    id: 'ollama',
    label: 'Ollama (local)',
    apiUrl: 'http://localhost:11434/v1/chat/completions',
    suggestedModel: 'llama3.3',
    roleModels: {
      reasoning: 'llama3.3',
      agentic: 'llama3.3',
      coding: 'qwen2.5-coder',
      summarization: 'llama3.2',
    },
    note: 'Local Ollama runtime via its OpenAI-compatible endpoint. API key is ignored — leave blank.',
    tint: {
      idle: 'border-stone-200 hover:border-stone-400 hover:bg-stone-50',
      selected: 'border-stone-500 bg-stone-200 ring-2 ring-stone-300 text-stone-900',
      dot: 'bg-stone-500',
    },
  },
  {
    id: 'custom',
    label: 'Custom',
    apiUrl: '',
    suggestedModel: '',
    roleModels: { ...EMPTY_ROLE_MODELS },
    note: 'Any other endpoint that speaks the OpenAI /chat/completions shape (vLLM, LiteLLM, LM Studio, self-hosted gateways).',
    tint: {
      idle: 'border-stone-200 hover:border-stone-400 hover:bg-stone-50',
      selected: 'border-stone-500 bg-stone-200 ring-2 ring-stone-300 text-stone-900',
      dot: 'bg-stone-400',
    },
  },
];

function detectPreset(apiUrl: string): ProviderPreset {
  const trimmed = apiUrl.trim();
  if (!trimmed) return PROVIDER_PRESETS[0];
  const match = PROVIDER_PRESETS.find(p => p.apiUrl && p.apiUrl === trimmed);
  if (match) return match;
  return PROVIDER_PRESETS[PROVIDER_PRESETS.length - 1]; // custom
}

/**
 * Configure the LLM provider (#1342). Defaults to the hosted OpenHuman
 * backend, whose built-in router picks the best model per request. Selecting
 * any other preset reveals per-role model inputs (reasoning / agentic /
 * coding / summarization) that get persisted to `config.model_routes` so the
 * core router obeys them.
 *
 * The api_key is stored on the user's machine in `config.toml`. It is sent
 * over the local Tauri↔core RPC only at write time; subsequent reads return
 * only `api_key_set: bool` so the secret never leaves the core process once
 * persisted.
 */
const BackendProviderPanel = () => {
  const { navigateBack, breadcrumbs } = useSettingsNavigation();
  const [loaded, setLoaded] = useState(false);
  const [client, setClient] = useState<ClientConfig | null>(null);
  const [apiUrl, setApiUrl] = useState('');
  const [apiKey, setApiKey] = useState('');
  const [apiKeyDirty, setApiKeyDirty] = useState(false);
  // Per-field dirty flags so a failed `load()` (which leaves the inputs at
  // their empty defaults) can't silently overwrite stored config when the
  // user clicks Save. CodeRabbit feedback on PR #1467.
  const [apiUrlDirty, setApiUrlDirty] = useState(false);
  const [roleModelsDirty, setRoleModelsDirty] = useState(false);
  const [roleModels, setRoleModels] = useState<RoleModels>(EMPTY_ROLE_MODELS);
  // Explicit active-preset state. We can't derive this from `apiUrl` alone
  // because OpenHuman and Custom both store an empty URL — clicking Custom
  // would otherwise snap back to OpenHuman on the next render.
  const [activePresetId, setActivePresetId] = useState<string>(PROVIDER_PRESETS[0].id);
  const [saving, setSaving] = useState(false);
  const [status, setStatus] = useState<{ kind: 'idle' | 'ok' | 'error'; message: string }>({
    kind: 'idle',
    message: '',
  });

  const load = useCallback(async () => {
    try {
      log('[llm-provider] loading client config');
      const response = await openhumanGetClientConfig();
      const config = response.result;
      setClient(config);
      const persistedUrl = config.api_url ?? '';
      setApiUrl(persistedUrl);
      setActivePresetId(detectPreset(persistedUrl).id);
      setApiKey('');
      setApiKeyDirty(false);
      setApiUrlDirty(false);
      setRoleModelsDirty(false);
      setLoaded(true);
    } catch (err) {
      log('failed to load client config: %s', err instanceof Error ? err.message : 'unknown');
      setStatus({
        kind: 'error',
        message:
          err instanceof Error
            ? `Failed to load current settings: ${err.message}`
            : 'Failed to load current settings.',
      });
      setLoaded(true);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const activePreset = useMemo(
    () => PROVIDER_PRESETS.find(p => p.id === activePresetId) ?? PROVIDER_PRESETS[0],
    [activePresetId]
  );
  const isOpenHuman = activePreset.id === 'openhuman';

  const applyPreset = useCallback((preset: ProviderPreset) => {
    setActivePresetId(preset.id);
    setApiUrl(preset.apiUrl);
    setApiUrlDirty(true);
    // Reset role models to the preset's defaults so each switch gives a
    // clean, opinionated starting point.
    setRoleModels(preset.roleModels ? { ...preset.roleModels } : { ...EMPTY_ROLE_MODELS });
    setRoleModelsDirty(true);
    setStatus({ kind: 'idle', message: '' });
  }, []);

  const handleSave = useCallback(async () => {
    setSaving(true);
    setStatus({ kind: 'idle', message: '' });
    try {
      // Build model_routes from role state when a non-OpenHuman preset is
      // active. Empty roles are filtered so the router doesn't dispatch to
      // an empty model id. Switching back to OpenHuman sends [] so the
      // built-in router takes over. We only send routes when the user has
      // actually changed the provider or edited a role input — keeps a
      // stale Save click after a failed `load()` from clobbering stored
      // config (CodeRabbit #1467).
      const routesTouched = apiUrlDirty || roleModelsDirty;
      const routes: ModelRoute[] | undefined = !routesTouched
        ? undefined
        : isOpenHuman
          ? []
          : ROLE_HINTS.flatMap(hint => {
              const model = roleModels[hint].trim();
              return model ? [{ hint, model }] : [];
            });
      await openhumanUpdateModelSettings({
        api_url: apiUrlDirty ? apiUrl : undefined,
        api_key: apiKeyDirty ? apiKey : undefined,
        model_routes: routes,
      });
      setStatus({ kind: 'ok', message: 'LLM provider settings saved.' });
      await load();
    } catch (err) {
      log('save failed: %s', err instanceof Error ? err.message : 'unknown');
      setStatus({
        kind: 'error',
        message:
          err instanceof Error ? `Failed to save: ${err.message}` : 'Failed to save settings.',
      });
    } finally {
      setSaving(false);
    }
  }, [apiKey, apiKeyDirty, apiUrl, apiUrlDirty, isOpenHuman, load, roleModels, roleModelsDirty]);

  const handleClearKey = useCallback(async () => {
    setSaving(true);
    setStatus({ kind: 'idle', message: '' });
    try {
      await openhumanUpdateModelSettings({ api_key: '' });
      setStatus({ kind: 'ok', message: 'API key cleared.' });
      await load();
    } catch (err) {
      log('clear key failed: %s', err instanceof Error ? err.message : 'unknown');
      setStatus({
        kind: 'error',
        message:
          err instanceof Error ? `Failed to clear key: ${err.message}` : 'Failed to clear key.',
      });
    } finally {
      setSaving(false);
    }
  }, [load]);

  return (
    <div>
      <SettingsHeader
        title="LLM Provider"
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />
      <div className="p-4 space-y-5">
        <p className="text-sm text-stone-500 leading-relaxed">
          Pick where inference runs. Any OpenAI-compatible provider (OpenAI, Anthropic, OpenRouter,
          Ollama, your own gateway).
        </p>

        {!loaded ? (
          <div className="text-sm text-stone-400 animate-pulse">Loading current settings…</div>
        ) : (
          <>
            <section className="space-y-2">
              <label className="block text-xs font-semibold uppercase tracking-wide text-stone-500">
                Provider
              </label>
              <div className="grid grid-cols-2 sm:grid-cols-3 gap-2">
                {PROVIDER_PRESETS.map(preset => {
                  const selected = preset.id === activePreset.id;
                  return (
                    <button
                      key={preset.id}
                      type="button"
                      onClick={() => applyPreset(preset)}
                      className={`flex items-center gap-2 rounded-lg border px-3 py-2 text-left transition-colors ${
                        selected ? preset.tint.selected : `bg-white ${preset.tint.idle}`
                      }`}
                      aria-pressed={selected}>
                      <span
                        className={`h-2 w-2 shrink-0 rounded-full ${preset.tint.dot}`}
                        aria-hidden="true"
                      />
                      <span className="text-sm font-medium text-stone-800 truncate">
                        {preset.label}
                      </span>
                    </button>
                  );
                })}
              </div>
              <p className="text-xs text-stone-400">{activePreset.note}</p>
            </section>

            {isOpenHuman && (
              <div className="rounded-lg border border-sage-300 bg-sage-50 p-3 flex gap-3">
                <div>
                  <p className="text-xs font-semibold uppercase tracking-wide text-sage-700">
                    Congrats! You’re using the most optimized setup
                  </p>
                  <p className="mt-1 text-sm text-sage-900 leading-relaxed">
                    OpenHuman's built-in smart router picks the best model giving you top-tier
                    quality at the lowest blended cost. All within your current subscription.
                  </p>
                </div>
              </div>
            )}

            {!isOpenHuman && (
              <div className="rounded-lg border border-primary-200 bg-primary-50 p-3">
                <p className="mt-1 text-sm text-primary-900 leading-relaxed">
                  Consider switching to OpenHuman as it comes with a built-in smart router that
                  picks the best model for each request, cutting costs and improving quality.
                </p>
              </div>
            )}

            {activePreset.id === 'custom' && (
              <section className="space-y-2">
                <label
                  htmlFor="llm-api-url"
                  className="block text-xs font-semibold uppercase tracking-wide text-stone-500">
                  API URL
                </label>
                <input
                  id="llm-api-url"
                  type="url"
                  value={apiUrl}
                  onChange={e => {
                    setApiUrl(e.target.value);
                    setApiUrlDirty(true);
                  }}
                  placeholder="https://your-host.example/v1/chat/completions"
                  className="w-full rounded-lg border border-stone-200 bg-white px-3 py-2 text-sm text-stone-900 placeholder:text-stone-400 focus:border-primary-400 focus:outline-none focus:ring-1 focus:ring-primary-200"
                  autoComplete="off"
                  spellCheck={false}
                />
                <p className="text-xs text-stone-400">
                  Full URL of the OpenAI-compatible chat-completions endpoint for your gateway.
                </p>
              </section>
            )}

            {!isOpenHuman && (
              <section className="space-y-2">
                <div className="flex items-center justify-between">
                  <label
                    htmlFor="llm-api-key"
                    className="block text-xs font-semibold uppercase tracking-wide text-stone-500">
                    API Key
                  </label>
                  {client?.api_key_set && (
                    <button
                      type="button"
                      onClick={handleClearKey}
                      disabled={saving}
                      className="text-xs text-coral-600 hover:text-coral-700 disabled:opacity-50">
                      Clear stored key
                    </button>
                  )}
                </div>
                <input
                  id="llm-api-key"
                  type="password"
                  value={apiKey}
                  onChange={e => {
                    setApiKey(e.target.value);
                    setApiKeyDirty(true);
                  }}
                  placeholder={
                    client?.api_key_set ? `${KEY_PLACEHOLDER} (replace to change)` : 'sk-…'
                  }
                  className="w-full rounded-lg border border-stone-200 bg-white px-3 py-2 text-sm text-stone-900 placeholder:text-stone-400 focus:border-primary-400 focus:outline-none focus:ring-1 focus:ring-primary-200"
                  autoComplete="off"
                  spellCheck={false}
                />
                <p className="text-xs text-stone-400">
                  {client?.api_key_set ? 'A key is currently saved.' : 'No key is currently saved.'}
                </p>
              </section>
            )}

            {!isOpenHuman && (
              <section className="space-y-2">
                <label className="block text-xs font-semibold uppercase tracking-wide text-stone-500">
                  Models by Role
                </label>
                <p className="text-xs text-stone-400">
                  The core router dispatches each task to the right model. Leave a field blank to
                  skip routing for that role.
                </p>
                <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
                  {ROLE_HINTS.map(hint => (
                    <div key={hint} className="space-y-1">
                      <label
                        htmlFor={`llm-role-${hint}`}
                        className="block text-xs font-medium text-stone-700">
                        {ROLE_LABELS[hint].label}
                      </label>
                      <input
                        id={`llm-role-${hint}`}
                        type="text"
                        value={roleModels[hint]}
                        onChange={e => {
                          const next = e.target.value;
                          setRoleModels(prev => ({ ...prev, [hint]: next }));
                          setRoleModelsDirty(true);
                        }}
                        placeholder={ROLE_LABELS[hint].help}
                        className="w-full rounded-lg border border-stone-200 bg-white px-3 py-2 text-sm text-stone-900 placeholder:text-stone-400 focus:border-primary-400 focus:outline-none focus:ring-1 focus:ring-primary-200"
                        autoComplete="off"
                        spellCheck={false}
                      />
                    </div>
                  ))}
                </div>
              </section>
            )}

            <div className="flex items-center gap-3 pt-1">
              <button
                type="button"
                onClick={handleSave}
                disabled={saving}
                className="rounded-lg bg-primary-500 px-4 py-2 text-sm font-medium text-white hover:bg-primary-600 disabled:opacity-50">
                {saving ? 'Saving…' : 'Save'}
              </button>
              {status.kind === 'ok' && (
                <span className="text-sm text-sage-700">{status.message}</span>
              )}
              {status.kind === 'error' && (
                <span className="text-sm text-coral-700">{status.message}</span>
              )}
            </div>
          </>
        )}
      </div>
    </div>
  );
};

export default BackendProviderPanel;
