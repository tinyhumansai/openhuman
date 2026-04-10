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

const MAX_LOG_ENTRIES = 200;

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

const AutocompleteDebugPanel = () => {
  const { navigateBack, breadcrumbs } = useSettingsNavigation();

  // Status & loading
  const [status, setStatus] = useState<AutocompleteStatus | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);

  // Advanced settings form state (dev-facing fields only)
  const [debounceMs, setDebounceMs] = useState<string>(String(DEFAULT_CONFIG.debounce_ms));
  const [maxChars, setMaxChars] = useState<string>(String(DEFAULT_CONFIG.max_chars));
  const [overlayTtlMs, setOverlayTtlMs] = useState<string>(String(DEFAULT_CONFIG.overlay_ttl_ms));
  const [styleInstructions, setStyleInstructions] = useState<string>('');
  const [styleExamplesText, setStyleExamplesText] = useState<string>('');

  // Test section
  const [contextOverride, setContextOverride] = useState<string>('');
  const [focusDebug, setFocusDebug] = useState<string>('');

  // Live logs
  const [logs, setLogs] = useState<string[]>([]);
  const previousStatusRef = useRef<AutocompleteStatus | null>(null);

  // Personalization history
  const [historyEntries, setHistoryEntries] = useState<AcceptedCompletion[]>([]);
  const [isHistoryLoading, setIsHistoryLoading] = useState(false);
  const [isClearingHistory, setIsClearingHistory] = useState(false);

  // -------------------------------------------------------------------------
  // Logging helpers
  // -------------------------------------------------------------------------

  const appendLogs = (entries: string[]) => {
    if (entries.length === 0) return;
    const now = new Date();
    const stamp = `${now.toLocaleTimeString()}.${String(now.getMilliseconds()).padStart(3, '0')}`;
    setLogs(current =>
      [...current, ...entries.map(entry => `${stamp}  ${entry}`)].slice(-MAX_LOG_ENTRIES)
    );
  };

  const appendUiLog = (entry: string) => {
    appendLogs([`[ui-flow] ${entry}`]);
  };

  const trackStatusChanges = (next: AutocompleteStatus) => {
    const previous = previousStatusRef.current;
    if (!previous) {
      previousStatusRef.current = next;
      appendLogs([
        `[runtime] phase=${next.phase} running=${next.running ? 'yes' : 'no'} enabled=${next.enabled ? 'yes' : 'no'}`,
      ]);
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

  // -------------------------------------------------------------------------
  // Data loading
  // -------------------------------------------------------------------------

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
      setDebounceMs(String(config.debounce_ms));
      setMaxChars(String(config.max_chars));
      setOverlayTtlMs(String(config.overlay_ttl_ms));
      setStyleInstructions(config.style_instructions ?? '');
      setStyleExamplesText(config.style_examples.join('\n'));
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to load autocomplete settings');
    } finally {
      setIsLoading(false);
    }
  };

  const loadHistory = async (): Promise<AcceptedCompletion[]> => {
    if (!isTauri()) return [];
    setIsHistoryLoading(true);
    try {
      const response = await openhumanAutocompleteHistory({ limit: 20 });
      setHistoryEntries(response.result.entries);
      return response.result.entries;
    } catch {
      // Non-critical — silently ignore
      return [];
    } finally {
      setIsHistoryLoading(false);
    }
  };

  useEffect(() => {
    void load();
    void loadHistory();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // -------------------------------------------------------------------------
  // Status polling
  // -------------------------------------------------------------------------

  const refreshStatus = async (showSpinner = false) => {
    if (!isTauri()) return null;
    if (showSpinner) {
      setIsLoading(true);
      setError(null);
    }
    try {
      const response = await openhumanAutocompleteStatus();
      setStatus(response.result);
      trackStatusChanges(response.result);
      if (showSpinner) {
        appendLogs(response.logs);
      }
      return response.result;
    } catch (err) {
      const msg = err instanceof Error ? err.message : 'Failed to refresh autocomplete status';
      appendUiLog(`refresh status failed: ${msg}`);
      setError(msg);
      return null;
    } finally {
      if (showSpinner) {
        setIsLoading(false);
      }
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

  // -------------------------------------------------------------------------
  // Runtime controls
  // -------------------------------------------------------------------------

  const start = async () => {
    if (!isTauri()) return;
    setError(null);
    setMessage(null);
    try {
      const debounce = Number(debounceMs);
      appendUiLog(`start requested (debounce=${String(debounce)}ms)`);
      const response = await openhumanAutocompleteStart({
        debounce_ms: Number.isFinite(debounce) ? Math.min(Math.max(debounce, 50), 2000) : 120,
      });
      appendLogs(response.logs);
      const latestStatus = await refreshStatus();
      if (response.result.started) {
        setMessage('Autocomplete started.');
      } else if (latestStatus?.enabled === false) {
        setMessage('Autocomplete is disabled in settings. Enable it and save first.');
      } else if (latestStatus?.running) {
        setMessage('Autocomplete is already running.');
      } else {
        setMessage('Autocomplete did not start.');
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : 'Failed to start autocomplete';
      appendUiLog(`start failed: ${msg}`);
      setError(msg);
    }
  };

  const stop = async () => {
    if (!isTauri()) return;
    setError(null);
    setMessage(null);
    try {
      appendUiLog('stop requested');
      const response = await openhumanAutocompleteStop({ reason: 'manual_stop_from_settings' });
      appendLogs(response.logs);
      const latestStatus = await refreshStatus();
      setMessage('Autocomplete stopped.');
      if (latestStatus?.running) {
        appendUiLog('runtime still reports running after stop');
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : 'Failed to stop autocomplete';
      appendUiLog(`stop failed: ${msg}`);
      setError(msg);
    }
  };

  // -------------------------------------------------------------------------
  // Test actions
  // -------------------------------------------------------------------------

  const testCurrent = async () => {
    if (!isTauri()) return;
    setError(null);
    setMessage(null);
    try {
      appendUiLog(
        contextOverride.trim()
          ? `get suggestion requested (override chars=${String(contextOverride.trim().length)})`
          : 'get suggestion requested (focused app context)'
      );
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
      const msg = err instanceof Error ? err.message : 'Failed to fetch current suggestion';
      appendUiLog(`get suggestion failed: ${msg}`);
      setError(msg);
    }
  };

  const waitForAcceptedHistoryEntry = async (acceptedValue?: string | null) => {
    if (!acceptedValue) {
      await loadHistory();
      return;
    }
    const normalized = acceptedValue.trim();
    if (!normalized) {
      await loadHistory();
      return;
    }

    const maxAttempts = 6;
    for (let attempt = 0; attempt < maxAttempts; attempt += 1) {
      const entries = await loadHistory();
      const found = entries.some(entry => entry.suggestion.trim() === normalized);
      if (found) {
        return;
      }
      if (attempt < maxAttempts - 1) {
        await new Promise(resolve => window.setTimeout(resolve, 180));
      }
    }
  };

  const acceptSuggestion = async () => {
    if (!isTauri()) return;
    setError(null);
    setMessage(null);
    try {
      appendUiLog('accept suggestion requested');
      const response = await openhumanAutocompleteAccept({
        suggestion: status?.suggestion?.value ?? undefined,
        skip_apply: true,
      });
      appendLogs(response.logs);
      if (response.result.accepted && response.result.value) {
        setMessage(`Accepted: ${response.result.value}`);
      } else {
        setMessage(response.result.reason ?? 'No suggestion was applied.');
      }
      await refreshStatus();
      await waitForAcceptedHistoryEntry(response.result.value);
    } catch (err) {
      const msg = err instanceof Error ? err.message : 'Failed to accept suggestion';
      appendUiLog(`accept failed: ${msg}`);
      setError(msg);
    }
  };

  const debugFocus = async () => {
    if (!isTauri()) return;
    setError(null);
    try {
      appendUiLog('debug focus requested');
      const response = await openhumanAutocompleteDebugFocus();
      appendLogs(response.logs);
      setFocusDebug(JSON.stringify(response.result, null, 2));
      appendUiLog(
        `focus app=${response.result.app_name ?? 'n/a'} role=${response.result.role ?? 'n/a'} chars=${String(response.result.context.length)}`
      );
    } catch (err) {
      const msg = err instanceof Error ? err.message : 'Failed to inspect focused element';
      appendUiLog(`debug focus failed: ${msg}`);
      setError(msg);
    }
  };

  // -------------------------------------------------------------------------
  // Advanced settings save
  // -------------------------------------------------------------------------

  const saveAdvancedConfig = async () => {
    if (!isTauri()) return;
    setIsSaving(true);
    setError(null);
    setMessage(null);
    try {
      appendUiLog('saving advanced autocomplete settings');
      const debounce = Number(debounceMs);
      const max = Number(maxChars);
      const ttl = Number(overlayTtlMs);
      const response = await openhumanAutocompleteSetStyle({
        debounce_ms: Number.isFinite(debounce) ? Math.min(Math.max(debounce, 50), 2000) : 120,
        max_chars: Number.isFinite(max) ? Math.min(Math.max(max, 32), 1200) : 384,
        overlay_ttl_ms: Number.isFinite(ttl) ? Math.min(Math.max(ttl, 300), 10000) : 1100,
        style_instructions: styleInstructions.trim() || undefined,
        style_examples: styleExamplesText
          .split('\n')
          .map(entry => entry.trim())
          .filter(Boolean),
      });
      setDebounceMs(String(response.result.config.debounce_ms));
      setMaxChars(String(response.result.config.max_chars));
      setOverlayTtlMs(String(response.result.config.overlay_ttl_ms));
      setStyleInstructions(response.result.config.style_instructions ?? '');
      setStyleExamplesText(response.result.config.style_examples.join('\n'));
      appendLogs(response.logs);
      setMessage('Advanced settings saved.');
      await refreshStatus();
    } catch (err) {
      const msg = err instanceof Error ? err.message : 'Failed to save advanced settings';
      appendUiLog(`save advanced settings failed: ${msg}`);
      setError(msg);
    } finally {
      setIsSaving(false);
    }
  };

  // -------------------------------------------------------------------------
  // History controls
  // -------------------------------------------------------------------------

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

  const clearLogs = () => {
    setLogs([]);
    previousStatusRef.current = status;
  };

  // -------------------------------------------------------------------------
  // Render
  // -------------------------------------------------------------------------

  return (
    <div className="z-10 relative">
      <SettingsHeader
        title="Autocomplete Debug"
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div className="max-w-2xl mx-auto w-full p-4 space-y-4">
        {/* ------------------------------------------------------------------ */}
        {/* Runtime section                                                     */}
        {/* ------------------------------------------------------------------ */}
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
              onClick={() => void refreshStatus(true)}
              disabled={isLoading}
              className="rounded-lg border border-stone-300 bg-stone-100 px-3 py-2 text-sm text-stone-700 disabled:opacity-50">
              {isLoading ? 'Refreshing…' : 'Refresh Status'}
            </button>
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

        {/* ------------------------------------------------------------------ */}
        {/* Test section                                                        */}
        {/* ------------------------------------------------------------------ */}
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

        {/* ------------------------------------------------------------------ */}
        {/* Live Logs section                                                   */}
        {/* ------------------------------------------------------------------ */}
        <section className="rounded-2xl border border-stone-200 bg-white p-4 space-y-3">
          <div className="flex items-center justify-between">
            <h3 className="text-sm font-semibold text-stone-900">Live Logs</h3>
            <button
              type="button"
              onClick={clearLogs}
              className="rounded-lg border border-stone-300 bg-stone-100 px-3 py-1.5 text-xs text-stone-700">
              Clear
            </button>
          </div>
          <pre className="max-h-56 overflow-auto rounded-xl border border-stone-200 bg-stone-50 p-2 text-xs text-stone-700">
            {logs.length > 0 ? logs.join('\n') : 'No logs yet.'}
          </pre>
        </section>

        {/* ------------------------------------------------------------------ */}
        {/* Advanced settings                                                   */}
        {/* ------------------------------------------------------------------ */}
        <section className="rounded-2xl border border-stone-200 bg-white p-4 space-y-3">
          <h3 className="text-sm font-semibold text-stone-900">Advanced Settings</h3>
          <label className="flex items-center justify-between rounded-xl border border-stone-200 bg-stone-50 px-3 py-2">
            <span className="text-sm text-stone-700">Debounce Ms</span>
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
            <span className="text-sm text-stone-700">Max Characters</span>
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
            <span className="text-sm text-stone-700">Overlay TTL Ms</span>
            <input
              type="number"
              min={300}
              max={10000}
              step={100}
              value={overlayTtlMs}
              onChange={event => setOverlayTtlMs(event.target.value)}
              className="w-28 rounded border border-stone-300 bg-white px-2 py-1 text-xs text-stone-700"
            />
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
          <button
            type="button"
            onClick={() => void saveAdvancedConfig()}
            disabled={isSaving}
            className="rounded-lg border border-primary-500/60 bg-primary-50 px-3 py-2 text-sm text-primary-600 disabled:opacity-50">
            {isSaving ? 'Saving…' : 'Save'}
          </button>
        </section>

        {/* ------------------------------------------------------------------ */}
        {/* Personalization History                                             */}
        {/* ------------------------------------------------------------------ */}
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
                  className="flex flex-col gap-0.5 rounded-lg bg-white px-2 py-1.5 text-xs border border-stone-100">
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

        {/* ------------------------------------------------------------------ */}
        {/* Feedback messages                                                   */}
        {/* ------------------------------------------------------------------ */}
        {message && <div className="text-xs text-green-700">{message}</div>}
        {error && <div className="text-xs text-red-600">{error}</div>}
      </div>
    </div>
  );
};

export default AutocompleteDebugPanel;
