import { useEffect, useRef, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
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
import Button from '../../ui/Button';
import Input from '../../ui/Input';
import { SettingsSection, SettingsStatusLine, SettingsTextArea } from '../controls';
import SettingsPanel from '../layout/SettingsPanel';

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
  const { t } = useT();

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
      setError(
        err instanceof Error ? err.message : t('settings.autocomplete.debug.loadSettingsFailed')
      );
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
      const msg =
        err instanceof Error ? err.message : t('settings.autocomplete.debug.refreshStatusFailed');
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
        setMessage(t('autocomplete.started'));
      } else if (latestStatus?.enabled === false) {
        setMessage(t('settings.autocomplete.debug.disabledInSettings'));
      } else if (latestStatus?.running) {
        setMessage(t('settings.autocomplete.debug.alreadyRunning'));
      } else {
        setMessage(t('settings.autocomplete.debug.didNotStart'));
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : t('settings.autocomplete.debug.startFailed');
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
      setMessage(t('autocomplete.stopped'));
      if (latestStatus?.running) {
        appendUiLog('runtime still reports running after stop');
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : t('settings.autocomplete.debug.stopFailed');
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
          ? t('settings.autocomplete.debug.suggestionPrefix').replace(
              '{value}',
              response.result.suggestion.value
            )
          : t('settings.autocomplete.debug.noSuggestionReturned')
      );
      await refreshStatus();
    } catch (err) {
      const msg =
        err instanceof Error ? err.message : t('settings.autocomplete.debug.fetchSuggestionFailed');
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
        setMessage(
          t('settings.autocomplete.debug.acceptedPrefix').replace('{value}', response.result.value)
        );
      } else {
        setMessage(response.result.reason ?? t('settings.autocomplete.debug.noSuggestionApplied'));
      }
      await refreshStatus();
      await waitForAcceptedHistoryEntry(response.result.value);
    } catch (err) {
      const msg =
        err instanceof Error ? err.message : t('settings.autocomplete.debug.acceptFailed');
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
      if (response.result) {
        appendUiLog(
          `focus app=${response.result.app_name ?? 'n/a'} role=${response.result.role ?? 'n/a'} chars=${String(response.result.context.length)}`
        );
      } else {
        appendUiLog('focus debug returned no focused element');
      }
    } catch (err) {
      const msg =
        err instanceof Error
          ? err.message
          : t('settings.autocomplete.debug.inspectFocusedElementFailed');
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
      setMessage(t('autocomplete.settingsSaved'));
      await refreshStatus();
    } catch (err) {
      const msg =
        err instanceof Error
          ? err.message
          : t('settings.autocomplete.debug.saveAdvancedSettingsFailed');
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
      setError(
        err instanceof Error ? err.message : t('settings.autocomplete.debug.clearHistoryFailed')
      );
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
    <SettingsPanel description={t('settings.developerMenu.autocomplete.desc')}>
      {/* ------------------------------------------------------------------ */}
      {/* Runtime section                                                     */}
      {/* ------------------------------------------------------------------ */}
      <SettingsSection title={t('settings.autocomplete.appFilter.runtime')}>
        <div className="px-4 py-3 space-y-3">
          <div className="text-sm text-content space-y-1">
            <div>
              {t('settings.autocomplete.appFilter.platformSupported')}:{' '}
              {status?.platform_supported ? t('common.yes') : t('common.no')}
            </div>
            <div>
              {t('settings.autocomplete.appFilter.enabled')}:{' '}
              {status?.enabled ? t('common.yes') : t('common.no')}
            </div>
            <div>
              {t('settings.autocomplete.appFilter.running')}:{' '}
              {status?.running ? t('common.yes') : t('common.no')}
            </div>
            <div>
              {t('settings.autocomplete.appFilter.phase')}:{' '}
              {status?.phase ?? t('settings.autocomplete.shared.unknown')}
            </div>
            <div>
              {t('settings.autocomplete.appFilter.debounce')}:{' '}
              {`${String(status?.debounce_ms ?? 0)}ms`}
            </div>
            <div>
              {t('settings.autocomplete.appFilter.model')}:{' '}
              {status?.model_id ?? t('settings.autocomplete.shared.notApplicable')}
            </div>
            <div>
              {t('settings.autocomplete.appFilter.app')}:{' '}
              {status?.app_name ?? t('settings.autocomplete.shared.notApplicable')}
            </div>
            <div>
              {t('settings.autocomplete.appFilter.lastError')}:{' '}
              {status?.last_error ?? t('settings.autocomplete.shared.none')}
            </div>
            <div>
              {t('settings.autocomplete.appFilter.currentSuggestion')}:{' '}
              {status?.suggestion?.value ?? t('settings.autocomplete.shared.none')}
            </div>
          </div>
          <div className="flex gap-2">
            <Button
              type="button"
              variant="secondary"
              size="sm"
              onClick={() => void refreshStatus(true)}
              disabled={isLoading}>
              {isLoading
                ? t('settings.autocomplete.appFilter.refreshing')
                : t('settings.autocomplete.appFilter.refreshStatus')}
            </Button>
            <Button
              type="button"
              variant="secondary"
              size="sm"
              onClick={() => void start()}
              disabled={!status?.platform_supported || Boolean(status?.running)}>
              {t('autocomplete.start')}
            </Button>
            <Button
              type="button"
              variant="secondary"
              tone="danger"
              size="sm"
              onClick={() => void stop()}
              disabled={!status?.running}>
              {t('autocomplete.stop')}
            </Button>
          </div>
        </div>
      </SettingsSection>

      {/* ------------------------------------------------------------------ */}
      {/* Test section                                                        */}
      {/* ------------------------------------------------------------------ */}
      <SettingsSection title={t('settings.autocomplete.appFilter.test')}>
        <div className="px-4 py-3 space-y-3">
          <div className="space-y-1">
            <div className="text-xs text-content-muted">
              {t('settings.autocomplete.appFilter.contextOverride')}
            </div>
            <SettingsTextArea
              value={contextOverride}
              onChange={event => setContextOverride(event.target.value)}
              rows={3}
              aria-label={t('settings.autocomplete.appFilter.contextOverride')}
            />
          </div>
          <div className="flex gap-2">
            <Button type="button" variant="secondary" size="sm" onClick={() => void testCurrent()}>
              {t('settings.autocomplete.appFilter.getSuggestion')}
            </Button>
            <Button
              type="button"
              variant="secondary"
              size="sm"
              onClick={() => void acceptSuggestion()}>
              {t('settings.autocomplete.appFilter.acceptSuggestion')}
            </Button>
            <Button type="button" variant="secondary" size="sm" onClick={() => void debugFocus()}>
              {t('settings.autocomplete.appFilter.debugFocus')}
            </Button>
          </div>
          {focusDebug && (
            <pre className="max-h-48 overflow-auto rounded-xl border border-line bg-surface-muted p-2 text-xs text-content">
              {focusDebug}
            </pre>
          )}
        </div>
      </SettingsSection>

      {/* ------------------------------------------------------------------ */}
      {/* Live Logs section                                                   */}
      {/* ------------------------------------------------------------------ */}
      <SettingsSection title={t('settings.autocomplete.appFilter.liveLogs')}>
        <div className="px-4 py-3 space-y-3">
          <div className="flex justify-end">
            <Button type="button" variant="secondary" size="xs" onClick={clearLogs}>
              {t('common.clear')}
            </Button>
          </div>
          {/* Bespoke log-stream display — kept intact */}
          <pre className="max-h-56 overflow-auto rounded-xl border border-line bg-surface-muted p-2 text-xs text-content">
            {logs.length > 0 ? logs.join('\n') : t('settings.autocomplete.appFilter.noLogs')}
          </pre>
        </div>
      </SettingsSection>

      {/* ------------------------------------------------------------------ */}
      {/* Advanced settings                                                   */}
      {/* ------------------------------------------------------------------ */}
      <SettingsSection title={t('autocomplete.advancedSettings')}>
        <div className="px-4 py-3 space-y-3">
          <label className="flex items-center justify-between rounded-xl border border-line bg-surface-muted px-3 py-2">
            <span className="text-sm text-content">
              {t('settings.autocomplete.completionStyle.debounce')}
            </span>
            <Input
              type="number"
              inputSize="sm"
              min={50}
              max={2000}
              step={10}
              value={debounceMs}
              onChange={event => setDebounceMs(event.target.value)}
              className="w-28"
              aria-label={t('settings.autocomplete.completionStyle.debounce')}
            />
          </label>
          <label className="flex items-center justify-between rounded-xl border border-line bg-surface-muted px-3 py-2">
            <span className="text-sm text-content">
              {t('settings.autocomplete.completionStyle.maxChars')}
            </span>
            <Input
              type="number"
              inputSize="sm"
              min={32}
              max={1200}
              step={8}
              value={maxChars}
              onChange={event => setMaxChars(event.target.value)}
              className="w-28"
              aria-label={t('settings.autocomplete.completionStyle.maxChars')}
            />
          </label>
          <label className="flex items-center justify-between rounded-xl border border-line bg-surface-muted px-3 py-2">
            <span className="text-sm text-content">
              {t('settings.autocomplete.completionStyle.overlayTtl')}
            </span>
            <Input
              type="number"
              inputSize="sm"
              min={300}
              max={10000}
              step={100}
              value={overlayTtlMs}
              onChange={event => setOverlayTtlMs(event.target.value)}
              className="w-28"
              aria-label={t('settings.autocomplete.completionStyle.overlayTtl')}
            />
          </label>
          <div className="space-y-1">
            <div className="text-xs text-content-muted">
              {t('settings.autocomplete.completionStyle.styleInstructions')}
            </div>
            <SettingsTextArea
              value={styleInstructions}
              onChange={event => setStyleInstructions(event.target.value)}
              rows={3}
              aria-label={t('settings.autocomplete.completionStyle.styleInstructions')}
            />
          </div>
          <div className="space-y-1">
            <div className="text-xs text-content-muted">
              {t('settings.autocomplete.completionStyle.styleExamples')}
            </div>
            <SettingsTextArea
              value={styleExamplesText}
              onChange={event => setStyleExamplesText(event.target.value)}
              rows={3}
              aria-label={t('settings.autocomplete.completionStyle.styleExamples')}
            />
          </div>
          <Button
            type="button"
            variant="primary"
            size="sm"
            onClick={() => void saveAdvancedConfig()}
            disabled={isSaving}>
            {isSaving ? t('autocomplete.saving') : t('common.save')}
          </Button>
        </div>
      </SettingsSection>

      {/* ------------------------------------------------------------------ */}
      {/* Personalization History                                             */}
      {/* ------------------------------------------------------------------ */}
      <SettingsSection title={t('settings.autocomplete.completionStyle.personalizationHistory')}>
        <div className="px-4 py-3 space-y-3">
          <div className="flex justify-end">
            <Button
              type="button"
              variant="secondary"
              tone="danger"
              size="xs"
              onClick={() => void clearHistory()}
              disabled={isClearingHistory || historyEntries.length === 0}>
              {isClearingHistory
                ? t('settings.autocomplete.completionStyle.clearing')
                : t('settings.autocomplete.completionStyle.clearHistory')}
            </Button>
          </div>
          <p className="text-xs text-content-muted">
            {isHistoryLoading
              ? t('common.loading')
              : historyEntries.length === 0
                ? t('settings.autocomplete.completionStyle.noHistory')
                : (historyEntries.length === 1
                    ? t('settings.autocomplete.completionStyle.acceptedCompletion')
                    : t('settings.autocomplete.completionStyle.acceptedCompletions')
                  ).replace('{count}', String(historyEntries.length))}
          </p>
          {/* Bespoke history list — kept intact */}
          {historyEntries.length > 0 && (
            <div className="max-h-48 overflow-y-auto rounded-xl border border-line bg-surface-muted p-2 space-y-1">
              {historyEntries.map((entry, idx) => (
                <div
                  key={`${String(entry.timestamp_ms)}-${String(idx)}`}
                  className="flex flex-col gap-0.5 rounded-lg bg-surface px-2 py-1.5 text-xs border border-line-subtle">
                  <div className="flex items-center gap-2 text-content-muted">
                    <span className="shrink-0">
                      {new Date(entry.timestamp_ms).toLocaleString()}
                    </span>
                    {entry.app_name && (
                      <span className="rounded bg-surface-subtle px-1 text-content-muted">
                        {entry.app_name}
                      </span>
                    )}
                  </div>
                  <div className="flex items-baseline gap-1 text-content truncate">
                    <span className="shrink-0 text-content-muted">…</span>
                    <span className="truncate text-content-muted">{entry.context.slice(-40)}</span>
                    <span className="shrink-0 text-content-muted">→</span>
                    <span className="font-medium text-primary-500 truncate">
                      {entry.suggestion}
                    </span>
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      </SettingsSection>

      {/* ------------------------------------------------------------------ */}
      {/* Feedback messages                                                   */}
      {/* ------------------------------------------------------------------ */}
      <SettingsStatusLine saving={false} savedNote={message} error={error} savingLabel="" />
    </SettingsPanel>
  );
};

export default AutocompleteDebugPanel;
