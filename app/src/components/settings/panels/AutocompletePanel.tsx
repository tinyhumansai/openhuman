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
import AppFilterSection from './autocomplete/AppFilterSection';
import CompletionStyleSection from './autocomplete/CompletionStyleSection';

const DEFAULT_CONFIG: AutocompleteConfig = {
  enabled: true,
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

const AutocompletePanel = () => {
  const { navigateBack, breadcrumbs } = useSettingsNavigation();
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
  const [overlayTtlMs, setOverlayTtlMs] = useState<string>(String(DEFAULT_CONFIG.overlay_ttl_ms));
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
      setOverlayTtlMs(String(config.overlay_ttl_ms));
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

  const saveConfig = async () => {
    if (!isTauri()) return;
    setIsSaving(true);
    setError(null);
    setMessage(null);
    try {
      appendUiLog('saving autocomplete settings');
      const debounce = Number(debounceMs);
      const max = Number(maxChars);
      const ttl = Number(overlayTtlMs);
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
        overlay_ttl_ms: Number.isFinite(ttl) ? Math.min(Math.max(ttl, 300), 10000) : 1100,
      });

      setEnabled(response.result.config.enabled);
      setDebounceMs(String(response.result.config.debounce_ms));
      setMaxChars(String(response.result.config.max_chars));
      setStylePreset(response.result.config.style_preset);
      setStyleInstructions(response.result.config.style_instructions ?? '');
      setStyleExamplesText(response.result.config.style_examples.join('\n'));
      setDisabledAppsText(response.result.config.disabled_apps.join('\n'));
      setAcceptWithTab(response.result.config.accept_with_tab);
      setOverlayTtlMs(String(response.result.config.overlay_ttl_ms));
      appendLogs(response.logs);
      setMessage('Autocomplete settings saved.');
      await refreshStatus();
    } catch (err) {
      const msg = err instanceof Error ? err.message : 'Failed to save autocomplete settings';
      appendUiLog(`save settings failed: ${msg}`);
      setError(msg);
    } finally {
      setIsSaving(false);
    }
  };

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

  useEffect(() => {
    if (!isTauri()) return;
    const intervalId = window.setInterval(() => {
      void refreshStatus();
    }, 1200);
    return () => window.clearInterval(intervalId);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const clearLogs = () => {
    setLogs([]);
    previousStatusRef.current = status;
  };

  return (
    <div className="z-10 relative">
      <SettingsHeader
        title="Inline Autocomplete"
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div className="max-w-2xl mx-auto w-full p-4 space-y-4">
        <AppFilterSection
          status={status}
          isLoading={isLoading}
          contextOverride={contextOverride}
          focusDebug={focusDebug}
          logs={logs}
          message={message}
          error={error}
          onSetContextOverride={setContextOverride}
          onRefreshStatus={() => void refreshStatus(true)}
          onStart={() => void start()}
          onStop={() => void stop()}
          onTestCurrent={() => void testCurrent()}
          onAcceptSuggestion={() => void acceptSuggestion()}
          onDebugFocus={() => void debugFocus()}
          onClearLogs={clearLogs}
        />

        <CompletionStyleSection
          enabled={enabled}
          debounceMs={debounceMs}
          maxChars={maxChars}
          stylePreset={stylePreset}
          styleInstructions={styleInstructions}
          styleExamplesText={styleExamplesText}
          disabledAppsText={disabledAppsText}
          acceptWithTab={acceptWithTab}
          overlayTtlMs={overlayTtlMs}
          isSaving={isSaving}
          historyEntries={historyEntries}
          isHistoryLoading={isHistoryLoading}
          isClearingHistory={isClearingHistory}
          onSetEnabled={setEnabled}
          onSetDebounceMs={setDebounceMs}
          onSetMaxChars={setMaxChars}
          onSetStylePreset={setStylePreset}
          onSetStyleInstructions={setStyleInstructions}
          onSetStyleExamplesText={setStyleExamplesText}
          onSetDisabledAppsText={setDisabledAppsText}
          onSetAcceptWithTab={setAcceptWithTab}
          onSetOverlayTtlMs={setOverlayTtlMs}
          onSaveConfig={() => void saveConfig()}
          onClearHistory={() => void clearHistory()}
        />
      </div>
    </div>
  );
};

export default AutocompletePanel;
