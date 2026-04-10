import { useEffect, useRef, useState } from 'react';

import {
  type AutocompleteConfig,
  type AutocompleteStatus,
  isTauri,
  openhumanAutocompleteSetStyle,
  openhumanAutocompleteStart,
  openhumanAutocompleteStatus,
  openhumanAutocompleteStop,
  openhumanGetConfig,
} from '../../../utils/tauriCommands';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const DEFAULT_CONFIG: AutocompleteConfig = {
  enabled: false,
  debounce_ms: 120,
  max_chars: 384,
  style_preset: 'balanced',
  style_instructions: null,
  style_examples: [],
  disabled_apps: [],
  accept_with_tab: true,
  overlay_ttl_ms: 1100,
};

const parseAutocompleteConfig = (raw: unknown): AutocompleteConfig => {
  if (!raw || typeof raw !== 'object') {
    return DEFAULT_CONFIG;
  }
  const value = raw as Record<string, unknown>;
  return {
    enabled: typeof value.enabled === 'boolean' ? value.enabled : DEFAULT_CONFIG.enabled,
    debounce_ms:
      typeof value.debounce_ms === 'number' ? value.debounce_ms : DEFAULT_CONFIG.debounce_ms,
    max_chars: typeof value.max_chars === 'number' ? value.max_chars : DEFAULT_CONFIG.max_chars,
    style_preset:
      typeof value.style_preset === 'string' ? value.style_preset : DEFAULT_CONFIG.style_preset,
    style_instructions:
      typeof value.style_instructions === 'string' ? value.style_instructions : null,
    style_examples: Array.isArray(value.style_examples)
      ? value.style_examples.filter((entry): entry is string => typeof entry === 'string')
      : DEFAULT_CONFIG.style_examples,
    disabled_apps: Array.isArray(value.disabled_apps)
      ? value.disabled_apps.filter((entry): entry is string => typeof entry === 'string')
      : DEFAULT_CONFIG.disabled_apps,
    accept_with_tab:
      typeof value.accept_with_tab === 'boolean'
        ? value.accept_with_tab
        : DEFAULT_CONFIG.accept_with_tab,
    overlay_ttl_ms:
      typeof value.overlay_ttl_ms === 'number'
        ? value.overlay_ttl_ms
        : DEFAULT_CONFIG.overlay_ttl_ms,
  };
};

const AutocompletePanel = () => {
  const { navigateBack, navigateToSettings, breadcrumbs } = useSettingsNavigation();
  const [status, setStatus] = useState<AutocompleteStatus | null>(null);
  const [isSaving, setIsSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);

  const [enabled, setEnabled] = useState<boolean>(DEFAULT_CONFIG.enabled);
  const [stylePreset, setStylePreset] = useState<string>(DEFAULT_CONFIG.style_preset);
  const [disabledAppsText, setDisabledAppsText] = useState<string>(
    DEFAULT_CONFIG.disabled_apps.join('\n')
  );
  const [acceptWithTab, setAcceptWithTab] = useState<boolean>(DEFAULT_CONFIG.accept_with_tab);

  // Hold full config so we can pass through unchanged advanced values on save
  const fullConfigRef = useRef<AutocompleteConfig>(DEFAULT_CONFIG);

  const load = async () => {
    if (!isTauri()) return;
    setError(null);
    try {
      const [statusResponse, configResponse] = await Promise.all([
        openhumanAutocompleteStatus(),
        openhumanGetConfig(),
      ]);
      setStatus(statusResponse.result);
      const config = parseAutocompleteConfig(
        (configResponse.result.config as Record<string, unknown> | undefined)?.autocomplete
      );
      fullConfigRef.current = config;
      setEnabled(config.enabled);
      setStylePreset(config.style_preset);
      setDisabledAppsText(config.disabled_apps.join('\n'));
      setAcceptWithTab(config.accept_with_tab);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to load autocomplete settings');
    }
  };

  useEffect(() => {
    void load();
  }, []);

  const refreshStatus = async () => {
    if (!isTauri()) return;
    try {
      const response = await openhumanAutocompleteStatus();
      setStatus(response.result);
    } catch {
      // Non-critical
    }
  };

  useEffect(() => {
    if (!isTauri()) return;
    const intervalId = window.setInterval(() => {
      void refreshStatus();
    }, 1200);
    return () => window.clearInterval(intervalId);
  }, []);

  const saveConfig = async () => {
    if (!isTauri()) return;
    setIsSaving(true);
    setError(null);
    setMessage(null);
    try {
      const prev = fullConfigRef.current;
      const response = await openhumanAutocompleteSetStyle({
        enabled,
        debounce_ms: prev.debounce_ms,
        max_chars: prev.max_chars,
        style_preset: stylePreset.trim() || 'balanced',
        style_instructions: prev.style_instructions ?? undefined,
        style_examples: prev.style_examples,
        disabled_apps: disabledAppsText
          .split('\n')
          .map(entry => entry.trim())
          .filter(Boolean),
        accept_with_tab: acceptWithTab,
        overlay_ttl_ms: prev.overlay_ttl_ms,
      });

      fullConfigRef.current = response.result.config;
      setEnabled(response.result.config.enabled);
      setStylePreset(response.result.config.style_preset);
      setDisabledAppsText(response.result.config.disabled_apps.join('\n'));
      setAcceptWithTab(response.result.config.accept_with_tab);
      setMessage('Autocomplete settings saved.');
      await refreshStatus();
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to save autocomplete settings');
    } finally {
      setIsSaving(false);
    }
  };

  const start = async () => {
    if (!isTauri()) return;
    setError(null);
    setMessage(null);
    try {
      const response = await openhumanAutocompleteStart({
        debounce_ms: fullConfigRef.current.debounce_ms,
      });
      await refreshStatus();
      if (response.result.started) {
        setMessage('Autocomplete started.');
      } else {
        setMessage('Autocomplete did not start. Check if it is enabled.');
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to start autocomplete');
    }
  };

  const stop = async () => {
    if (!isTauri()) return;
    setError(null);
    setMessage(null);
    try {
      await openhumanAutocompleteStop({ reason: 'manual_stop_from_settings' });
      await refreshStatus();
      setMessage('Autocomplete stopped.');
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to stop autocomplete');
    }
  };

  return (
    <div className="z-10 relative">
      <SettingsHeader
        title="Autocomplete"
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div className="max-w-2xl mx-auto w-full p-4 space-y-4">
        <section className="rounded-2xl border border-stone-200 bg-white p-4 space-y-3">
          <h3 className="text-sm font-semibold text-stone-900">Settings</h3>

          <label className="flex items-center justify-between rounded-xl border border-stone-200 bg-stone-50 px-3 py-2">
            <span className="text-sm text-stone-700">Enabled</span>
            <input
              type="checkbox"
              checked={enabled}
              onChange={event => setEnabled(event.target.checked)}
            />
          </label>

          <label className="flex items-center justify-between rounded-xl border border-stone-200 bg-stone-50 px-3 py-2">
            <span className="text-sm text-stone-700">Accept With Tab</span>
            <input
              type="checkbox"
              checked={acceptWithTab}
              onChange={event => setAcceptWithTab(event.target.checked)}
            />
          </label>

          <label className="flex items-center justify-between rounded-xl border border-stone-200 bg-stone-50 px-3 py-2">
            <span className="text-sm text-stone-700">Style Preset</span>
            <select
              value={stylePreset}
              onChange={event => setStylePreset(event.target.value)}
              className="rounded border border-stone-300 bg-white px-2 py-1 text-xs text-stone-700">
              <option value="balanced">Balanced</option>
              <option value="concise">Concise</option>
              <option value="formal">Formal</option>
              <option value="casual">Casual</option>
              <option value="custom">Custom</option>
            </select>
          </label>

          <div className="space-y-1">
            <div className="text-xs text-stone-600">
              Disabled Apps (one bundle/app token per line)
            </div>
            <textarea
              value={disabledAppsText}
              onChange={event => setDisabledAppsText(event.target.value)}
              rows={3}
              className="w-full rounded border border-stone-200 bg-stone-50 p-2 text-xs text-stone-700"
            />
          </div>

          <button
            type="button"
            onClick={() => void saveConfig()}
            disabled={isSaving}
            className="rounded-lg border border-primary-500/60 bg-primary-50 px-3 py-2 text-sm text-primary-600 disabled:opacity-50">
            {isSaving ? 'Saving…' : 'Save Settings'}
          </button>
        </section>

        <section className="rounded-2xl border border-stone-200 bg-white p-4 space-y-3">
          <h3 className="text-sm font-semibold text-stone-900">Runtime</h3>
          <div className="text-sm text-stone-600 space-y-1">
            <div>Running: {status?.running ? 'yes' : 'no'}</div>
            <div>Enabled: {status?.enabled ? 'yes' : 'no'}</div>
          </div>
          <div className="flex gap-2">
            <button
              type="button"
              onClick={() => void start()}
              disabled={!status?.platform_supported || Boolean(status?.running)}
              className="rounded-lg border border-green-500/60 bg-green-50 px-3 py-2 text-sm text-green-700 disabled:opacity-50">
              Start
            </button>
            <button
              type="button"
              onClick={() => void stop()}
              disabled={!status?.running}
              className="rounded-lg border border-red-500/60 bg-red-50 px-3 py-2 text-sm text-red-600 disabled:opacity-50">
              Stop
            </button>
          </div>
        </section>

        {message && <div className="text-xs text-green-700">{message}</div>}
        {error && <div className="text-xs text-red-600">{error}</div>}

        <button
          type="button"
          onClick={() => navigateToSettings('autocomplete-debug')}
          className="flex items-center gap-1.5 text-xs text-stone-400 hover:text-stone-600 transition-colors">
          Advanced settings
          <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
          </svg>
        </button>
      </div>
    </div>
  );
};

export default AutocompletePanel;
