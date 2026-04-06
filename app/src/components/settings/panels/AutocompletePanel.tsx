import { useEffect, useRef, useState } from 'react';

import {
  type AcceptedCompletion,
  type AutocompleteConfig,
  type AutocompleteStatus,
  isTauri,
  openhumanAutocompleteAccept,
  openhumanAutocompleteClearHistory,
  openhumanAutocompleteCurrent,
  openhumanAutocompleteDebugFocus,
  openhumanAutocompleteHistory,
  openhumanAutocompleteSetStyle,
  openhumanAutocompleteStart,
  openhumanAutocompleteStatus,
  openhumanAutocompleteStop,
  openhumanGetConfig,
} from '../../../utils/tauriCommands';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const DEFAULT_CONFIG: AutocompleteConfig = {
  enabled: true,
  debounce_ms: 120,
  max_chars: 384,
  style_preset: 'balanced',
  style_instructions: null,
  style_examples: [],
  disabled_apps: [],
  accept_with_tab: true,
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
  };
};

const AutocompletePanel = () => {
  const { navigateBack } = useSettingsNavigation();
  const [status, setStatus] = useState<AutocompleteStatus | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);

  const [enabled, setEnabled] = useState<boolean>(DEFAULT_CONFIG.enabled);
  const [debounceMs, setDebounceMs] = useState<string>(String(DEFAULT_CONFIG.debounce_ms));
  const [maxChars, setMaxChars] = useState<string>(String(DEFAULT_CONFIG.max_chars));
  const [stylePreset, setStylePreset] = useState<string>(DEFAULT_CONFIG.style_preset);
  const [styleInstructions, setStyleInstructions] = useState<string>('');
  const [styleExamplesText, setStyleExamplesText] = useState<string>('');
  const [disabledAppsText, setDisabledAppsText] = useState<string>(
    DEFAULT_CONFIG.disabled_apps.join('\n')
  );
  const [acceptWithTab, setAcceptWithTab] = useState<boolean>(DEFAULT_CONFIG.accept_with_tab);
  const [contextOverride, setContextOverride] = useState<string>('');
  const [focusDebug, setFocusDebug] = useState<string>('');
  const [logs, setLogs] = useState<string[]>([]);
  const previousStatusRef = useRef<AutocompleteStatus | null>(null);

  // Personalization history state
  const [historyEntries, setHistoryEntries] = useState<AcceptedCompletion[]>([]);
  const [isHistoryLoading, setIsHistoryLoading] = useState(false);
  const [isClearingHistory, setIsClearingHistory] = useState(false);

  const appendLogs = (entries: string[]) => {
    if (entries.length === 0) return;
    const now = new Date().toLocaleTimeString();
    setLogs(current => [...current, ...entries.map(entry => `${now}  ${entry}`)].slice(-120));
  };

  const trackStatusChanges = (next: AutocompleteStatus) => {
    const previous = previousStatusRef.current;
    if (!previous) {
      previousStatusRef.current = next;
      appendLogs([`phase=${next.phase}`]);
      return;
    }

    const nextEntries: string[] = [];
    if (next.phase !== previous.phase) {
      nextEntries.push(`phase ${previous.phase} -> ${next.phase}`);
    }
    if ((next.last_error ?? '') !== (previous.last_error ?? '') && next.last_error) {
      nextEntries.push(`error: ${next.last_error}`);
    }
    if (
      (next.suggestion?.value ?? '') !== (previous.suggestion?.value ?? '') &&
      next.suggestion?.value
    ) {
      nextEntries.push(`suggestion ready: "${next.suggestion.value}"`);
    }

    if (nextEntries.length > 0) {
      appendLogs(nextEntries);
    }
    previousStatusRef.current = next;
  };

  const load = async () => {
    if (!isTauri()) return;
    setIsLoading(true);
    setError(null);
    try {
      const [statusResponse, configResponse] = await Promise.all([
        openhumanAutocompleteStatus(),
        openhumanGetConfig(),
      ]);
      setStatus(statusResponse.result);
      trackStatusChanges(statusResponse.result);
      appendLogs(statusResponse.logs);
      const config = parseAutocompleteConfig(
        (configResponse.result.config as Record<string, unknown> | undefined)?.autocomplete
      );
      setEnabled(config.enabled);
      setDebounceMs(String(config.debounce_ms));
      setMaxChars(String(config.max_chars));
      setStylePreset(config.style_preset);
      setStyleInstructions(config.style_instructions ?? '');
      setStyleExamplesText(config.style_examples.join('\n'));
      setDisabledAppsText(config.disabled_apps.join('\n'));
      setAcceptWithTab(config.accept_with_tab);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to load autocomplete settings');
    } finally {
      setIsLoading(false);
    }
  };

  useEffect(() => {
    void load();
    void loadHistory();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const loadHistory = async () => {
    if (!isTauri()) return;
    setIsHistoryLoading(true);
    try {
      const response = await openhumanAutocompleteHistory({ limit: 20 });
      setHistoryEntries(response.result.entries);
    } catch {
      // Non-critical — silently ignore
    } finally {
      setIsHistoryLoading(false);
    }
  };

  const clearHistory = async () => {
    if (!isTauri()) return;
    setIsClearingHistory(true);
    try {
      await openhumanAutocompleteClearHistory();
      setHistoryEntries([]);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to clear history');
    } finally {
      setIsClearingHistory(false);
    }
  };

  const refreshStatus = async () => {
    if (!isTauri()) return;
    try {
      const response = await openhumanAutocompleteStatus();
      setStatus(response.result);
      trackStatusChanges(response.result);
      appendLogs(response.logs);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to refresh autocomplete status');
    }
  };

  const saveConfig = async () => {
    if (!isTauri()) return;
    setIsSaving(true);
    setError(null);
    setMessage(null);
    try {
      const debounce = Number(debounceMs);
      const max = Number(maxChars);
      const response = await openhumanAutocompleteSetStyle({
        enabled,
        debounce_ms: Number.isFinite(debounce) ? Math.min(Math.max(debounce, 50), 2000) : 120,
        max_chars: Number.isFinite(max) ? Math.min(Math.max(max, 32), 1200) : 384,
        style_preset: stylePreset.trim() || 'balanced',
        style_instructions: styleInstructions.trim() || undefined,
        style_examples: styleExamplesText
          .split('\n')
          .map(entry => entry.trim())
          .filter(Boolean),
        disabled_apps: disabledAppsText
          .split('\n')
          .map(entry => entry.trim())
          .filter(Boolean),
        accept_with_tab: acceptWithTab,
      });

      setEnabled(response.result.config.enabled);
      setDebounceMs(String(response.result.config.debounce_ms));
      setMaxChars(String(response.result.config.max_chars));
      setStylePreset(response.result.config.style_preset);
      setStyleInstructions(response.result.config.style_instructions ?? '');
      setStyleExamplesText(response.result.config.style_examples.join('\n'));
      setDisabledAppsText(response.result.config.disabled_apps.join('\n'));
      setAcceptWithTab(response.result.config.accept_with_tab);
      appendLogs(response.logs);
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
    try {
      const debounce = Number(debounceMs);
      const response = await openhumanAutocompleteStart({
        debounce_ms: Number.isFinite(debounce) ? Math.min(Math.max(debounce, 50), 2000) : 120,
      });
      appendLogs(response.logs);
      await refreshStatus();
      setMessage('Autocomplete started.');
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to start autocomplete');
    }
  };

  const stop = async () => {
    if (!isTauri()) return;
    setError(null);
    try {
      const response = await openhumanAutocompleteStop({ reason: 'manual_stop_from_settings' });
      appendLogs(response.logs);
      await refreshStatus();
      setMessage('Autocomplete stopped.');
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to stop autocomplete');
    }
  };

  const testCurrent = async () => {
    if (!isTauri()) return;
    setError(null);
    try {
      const response = await openhumanAutocompleteCurrent({
        context: contextOverride.trim() || undefined,
      });
      appendLogs(response.logs);
      setMessage(
        response.result.suggestion?.value
          ? `Suggestion: ${response.result.suggestion.value}`
          : 'No suggestion returned.'
      );
      await refreshStatus();
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to fetch current suggestion');
    }
  };

  const acceptSuggestion = async () => {
    if (!isTauri()) return;
    setError(null);
    try {
      const response = await openhumanAutocompleteAccept();
      appendLogs(response.logs);
      if (response.result.applied && response.result.value) {
        setMessage(`Accepted: ${response.result.value}`);
      } else {
        setMessage(response.result.reason ?? 'No suggestion was applied.');
      }
      await refreshStatus();
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to accept suggestion');
    }
  };

  const debugFocus = async () => {
    if (!isTauri()) return;
    setError(null);
    try {
      const response = await openhumanAutocompleteDebugFocus();
      appendLogs(response.logs);
      setFocusDebug(JSON.stringify(response.result, null, 2));
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to inspect focused element');
    }
  };

  useEffect(() => {
    if (!isTauri()) return;
    const intervalId = window.setInterval(() => {
      void refreshStatus();
    }, 1200);
    return () => window.clearInterval(intervalId);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <div className="z-10 relative">
      <SettingsHeader title="Inline Autocomplete" showBackButton={true} onBack={navigateBack} />

      <div className="max-w-2xl mx-auto w-full p-4 space-y-4">
        <section className="rounded-2xl border border-stone-200 bg-white p-4 space-y-3">
          <h3 className="text-sm font-semibold text-stone-900">Runtime</h3>
          <div className="text-sm text-stone-700 space-y-1">
            <div>Platform supported: {status?.platform_supported ? 'yes' : 'no'}</div>
            <div>Enabled: {status?.enabled ? 'yes' : 'no'}</div>
            <div>Running: {status?.running ? 'yes' : 'no'}</div>
            <div>Phase: {status?.phase ?? 'unknown'}</div>
            <div>Debounce: {status?.debounce_ms ?? 0}ms</div>
            <div>Model: {status?.model_id ?? 'n/a'}</div>
            <div>App: {status?.app_name ?? 'n/a'}</div>
            <div>Last error: {status?.last_error ?? 'none'}</div>
            <div>Current suggestion: {status?.suggestion?.value ?? 'none'}</div>
          </div>
          <div className="flex gap-2">
            <button
              type="button"
              onClick={() => void refreshStatus()}
              disabled={isLoading}
              className="rounded-lg border border-stone-300 bg-stone-100 px-3 py-2 text-sm text-stone-700 disabled:opacity-50">
              {isLoading ? 'Refreshing…' : 'Refresh Status'}
            </button>
            <button
              type="button"
              onClick={() => void start()}
              className="rounded-lg border border-green-500/60 bg-green-50 px-3 py-2 text-sm text-green-700 disabled:opacity-50">
              Start
            </button>
            <button
              type="button"
              onClick={() => void stop()}
              className="rounded-lg border border-red-500/60 bg-red-50 px-3 py-2 text-sm text-red-600 disabled:opacity-50">
              Stop
            </button>
          </div>
        </section>

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
            <span className="text-sm text-stone-700">Debounce (ms)</span>
            <input
              type="number"
              min={50}
              max={2000}
              step={10}
              value={debounceMs}
              onChange={event => setDebounceMs(event.target.value)}
              className="w-28 rounded border border-stone-300 bg-white px-2 py-1 text-xs text-stone-700"
            />
          </label>
          <label className="flex items-center justify-between rounded-xl border border-stone-200 bg-stone-50 px-3 py-2">
            <span className="text-sm text-stone-700">Max Chars</span>
            <input
              type="number"
              min={32}
              max={1200}
              step={8}
              value={maxChars}
              onChange={event => setMaxChars(event.target.value)}
              className="w-28 rounded border border-stone-300 bg-white px-2 py-1 text-xs text-stone-700"
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
            <div className="text-xs text-stone-600">Style Instructions</div>
            <textarea
              value={styleInstructions}
              onChange={event => setStyleInstructions(event.target.value)}
              rows={3}
              className="w-full rounded border border-stone-200 bg-stone-50 p-2 text-xs text-stone-700"
            />
          </div>
          <div className="space-y-1">
            <div className="text-xs text-stone-600">Style Examples (one per line)</div>
            <textarea
              value={styleExamplesText}
              onChange={event => setStyleExamplesText(event.target.value)}
              rows={3}
              className="w-full rounded border border-stone-200 bg-stone-50 p-2 text-xs text-stone-700"
            />
          </div>
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
            {isSaving ? 'Saving…' : 'Save Autocomplete Settings'}
          </button>
        </section>

        <section className="rounded-2xl border border-stone-200 bg-white p-4 space-y-3">
          <div className="flex items-center justify-between">
            <h3 className="text-sm font-semibold text-stone-900">Personalization History</h3>
            <button
              type="button"
              onClick={() => void clearHistory()}
              disabled={isClearingHistory || historyEntries.length === 0}
              className="rounded-lg border border-red-500/60 bg-red-50 px-3 py-1.5 text-xs text-red-600 disabled:opacity-40">
              {isClearingHistory ? 'Clearing…' : 'Clear History'}
            </button>
          </div>
          <p className="text-xs text-stone-500">
            {isHistoryLoading
              ? 'Loading…'
              : historyEntries.length === 0
                ? 'No accepted completions yet. Accept suggestions with Tab to start personalising.'
                : `${String(historyEntries.length)} accepted completion${historyEntries.length === 1 ? '' : 's'} stored — used to personalise future suggestions.`}
          </p>
          {historyEntries.length > 0 && (
            <div className="max-h-48 overflow-y-auto rounded-xl border border-stone-200 bg-stone-50 p-2 space-y-1">
              {historyEntries.map((entry, idx) => (
                <div
                  key={`${String(entry.timestamp_ms)}-${String(idx)}`}
                  className="flex flex-col gap-0.5 rounded-lg bg-white px-2 py-1.5 text-xs border border-stone-100 text-xs">
                  <div className="flex items-center gap-2 text-stone-500">
                    <span className="shrink-0">
                      {new Date(entry.timestamp_ms).toLocaleString()}
                    </span>
                    {entry.app_name && (
                      <span className="rounded bg-stone-100 px-1 text-stone-600">
                        {entry.app_name}
                      </span>
                    )}
                  </div>
                  <div className="flex items-baseline gap-1 text-stone-700 truncate">
                    <span className="shrink-0 text-stone-400">…</span>
                    <span className="truncate text-stone-500">{entry.context.slice(-40)}</span>
                    <span className="shrink-0 text-stone-400">→</span>
                    <span className="font-medium text-primary-500 truncate">
                      {entry.suggestion}
                    </span>
                  </div>
                </div>
              ))}
            </div>
          )}
        </section>

        <section className="rounded-2xl border border-stone-200 bg-white p-4 space-y-3">
          <h3 className="text-sm font-semibold text-stone-900">Test</h3>
          <div className="space-y-1">
            <div className="text-xs text-stone-600">Context Override (optional)</div>
            <textarea
              value={contextOverride}
              onChange={event => setContextOverride(event.target.value)}
              rows={3}
              className="w-full rounded border border-stone-200 bg-stone-50 p-2 text-xs text-stone-700"
            />
          </div>
          <div className="flex gap-2">
            <button
              type="button"
              onClick={() => void testCurrent()}
              className="rounded-lg border border-primary-500/60 bg-primary-50 px-3 py-2 text-sm text-primary-600">
              Get Suggestion
            </button>
            <button
              type="button"
              onClick={() => void acceptSuggestion()}
              className="rounded-lg border border-emerald-500/60 bg-emerald-50 px-3 py-2 text-sm text-emerald-700">
              Accept Suggestion
            </button>
            <button
              type="button"
              onClick={() => void debugFocus()}
              className="rounded-lg border border-amber-500/60 bg-amber-50 px-3 py-2 text-sm text-amber-700">
              Debug Focus
            </button>
          </div>
          {focusDebug && (
            <pre className="max-h-48 overflow-auto rounded-xl border border-stone-200 bg-stone-50 p-2 text-xs text-stone-700">
              {focusDebug}
            </pre>
          )}
        </section>

        <section className="rounded-2xl border border-stone-200 bg-white p-4 space-y-3">
          <div className="flex items-center justify-between">
            <h3 className="text-sm font-semibold text-stone-900">Live Logs</h3>
            <button
              type="button"
              onClick={() => setLogs([])}
              className="rounded-lg border border-stone-300 bg-stone-100 px-3 py-1.5 text-xs text-stone-700">
              Clear
            </button>
          </div>
          <pre className="max-h-56 overflow-auto rounded-xl border border-stone-200 bg-stone-50 p-2 text-xs text-stone-700">
            {logs.length > 0 ? logs.join('\n') : 'No logs yet.'}
          </pre>
        </section>

        {message && <div className="text-xs text-green-700">{message}</div>}
        {error && <div className="text-xs text-red-600">{error}</div>}
      </div>
    </div>
  );
};

export default AutocompletePanel;
