import { useCallback, useEffect, useMemo, useState } from 'react';

import { useBackendUrl } from '../../../hooks/useBackendUrl';
import { useT } from '../../../lib/i18n/I18nContext';
import { tunnelsApi } from '../../../services/api/tunnelsApi';
import {
  buildWebhookEventsUrl,
  getCoreHttpBaseUrl,
  getCoreRpcToken,
} from '../../../services/coreRpcClient';
import {
  openhumanWebhooksClearLogs,
  openhumanWebhooksListLogs,
  openhumanWebhooksListRegistrations,
  type WebhookDebugEvent,
  type WebhookDebugLogEntry,
  type WebhookDebugRegistration,
} from '../../../utils/tauriCommands';
import PanelPage from '../../layout/PanelPage';
import Button from '../../ui/Button';
import SettingsBackButton from '../components/SettingsBackButton';
import {
  SettingsBadge,
  SettingsEmptyState,
  SettingsSection,
  SettingsStatusLine,
} from '../controls';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const LOG_LIMIT = 100;

function formatDateTime(timestamp: number): string {
  if (!timestamp) return '-';
  return new Date(timestamp).toLocaleString();
}

function decodeBase64Preview(value: string): string {
  if (!value) return '';
  try {
    return atob(value);
  } catch {
    return '[binary or invalid base64 payload]';
  }
}

function prettyJson(value: unknown): string {
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

const WebhooksDebugPanel = () => {
  const { t } = useT();
  const { navigateBack } = useSettingsNavigation();
  const backendUrl = useBackendUrl();
  const [registrations, setRegistrations] = useState<WebhookDebugRegistration[]>([]);
  const [logs, setLogs] = useState<WebhookDebugLogEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [clearing, setClearing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selectedCorrelationId, setSelectedCorrelationId] = useState<string | null>(null);
  const [isLive, setIsLive] = useState(false);
  const [lastEvent, setLastEvent] = useState<WebhookDebugEvent | null>(null);

  const loadData = useCallback(async () => {
    setError(null);
    try {
      const [registrationsResponse, logsResponse] = await Promise.all([
        openhumanWebhooksListRegistrations(),
        openhumanWebhooksListLogs(LOG_LIMIT),
      ]);
      setRegistrations(registrationsResponse.result.result.registrations);
      setLogs(logsResponse.result.result.logs);
      setSelectedCorrelationId(current =>
        current && logsResponse.result.result.logs.some(log => log.correlation_id === current)
          ? current
          : (logsResponse.result.result.logs[0]?.correlation_id ?? null)
      );
    } catch (loadError) {
      setError(
        loadError instanceof Error ? loadError.message : t('webhooks.failedToLoadDebugData')
      );
    } finally {
      setLoading(false);
    }
  }, [t]);

  useEffect(() => {
    void loadData();
  }, [loadData]);

  useEffect(() => {
    let eventSource: EventSource | null = null;
    let cancelled = false;

    const connect = async () => {
      try {
        const [baseUrl, coreRpcToken] = await Promise.all([
          getCoreHttpBaseUrl(),
          getCoreRpcToken(),
        ]);
        if (cancelled) return;

        const url = buildWebhookEventsUrl(baseUrl, coreRpcToken);
        if (!url) {
          // No bearer available — skip rather than open an unauth request
          // that the server will 401 and EventSource will reconnect to forever.
          setIsLive(false);
          return;
        }
        eventSource = new EventSource(url);

        eventSource.addEventListener('webhooks_debug', event => {
          setIsLive(true);
          try {
            setLastEvent(JSON.parse((event as MessageEvent<string>).data) as WebhookDebugEvent);
          } catch {
            setLastEvent(null);
          }
          void loadData();
        });

        eventSource.onerror = () => {
          setIsLive(false);
        };
      } catch {
        setIsLive(false);
      }
    };

    void connect();

    return () => {
      cancelled = true;
      if (eventSource) {
        eventSource.close();
      }
      setIsLive(false);
    };
  }, [loadData]);

  const selectedLog = useMemo(
    () => logs.find(entry => entry.correlation_id === selectedCorrelationId) ?? logs[0] ?? null,
    [logs, selectedCorrelationId]
  );

  const handleClearLogs = useCallback(async () => {
    const confirmed = window.confirm(t('webhooks.clearLogsConfirm'));
    if (!confirmed) return;

    setClearing(true);
    setError(null);
    try {
      await openhumanWebhooksClearLogs();
      await loadData();
    } catch (clearError) {
      setError(clearError instanceof Error ? clearError.message : t('webhooks.failedToClearLogs'));
    } finally {
      setClearing(false);
    }
  }, [loadData, t]);

  return (
    <PanelPage
      className="z-10"
      contentClassName=""
      testId="webhooks-debug-panel"
      description={t('settings.developerMenu.webhooks.desc')}
      leading={<SettingsBackButton onBack={navigateBack} />}>
      <div className="p-4 space-y-4">
        {/* Status bar */}
        <div className="flex flex-wrap items-center gap-2">
          <Button
            type="button"
            variant="secondary"
            size="xs"
            onClick={() => void loadData()}
            disabled={loading}>
            {loading ? t('webhooks.loading') : t('webhooks.refresh')}
          </Button>
          <Button
            type="button"
            variant="secondary"
            size="xs"
            onClick={() => void handleClearLogs()}
            disabled={clearing || logs.length === 0}>
            {clearing ? t('webhooks.clearing') : t('webhooks.clearLogs')}
          </Button>
          <span className="text-xs text-neutral-500 dark:text-neutral-400">
            {registrations.length} {t('webhooks.registered')} &middot; {logs.length}{' '}
            {t('webhooks.captured')} &middot;{' '}
            <span
              className={
                isLive
                  ? 'text-sage-600 dark:text-sage-300'
                  : 'text-neutral-500 dark:text-neutral-400'
              }>
              {isLive ? t('webhooks.live') : t('webhooks.disconnected')}
            </span>
          </span>
        </div>

        <SettingsStatusLine saving={false} error={error} savingLabel="" />

        {lastEvent && (
          <div className="text-xs text-neutral-500 dark:text-neutral-400">
            {t('webhooks.lastEvent')}:{' '}
            <span className="font-medium text-neutral-800 dark:text-neutral-200">
              {lastEvent.event_type}
            </span>{' '}
            {t('webhooks.at')} {formatDateTime(lastEvent.timestamp)}
          </div>
        )}

        {/* Registrations */}
        <SettingsSection title={t('webhooks.registeredWebhooks')}>
          <div className="px-4 py-3 space-y-2">
            {registrations.length === 0 ? (
              <SettingsEmptyState label={t('webhooks.noActiveRegistrations')} />
            ) : (
              <div className="space-y-2">
                {registrations.map(registration => (
                  <div
                    key={registration.tunnel_uuid}
                    className="rounded-xl border border-neutral-200 dark:border-neutral-800 bg-neutral-50 dark:bg-neutral-800/60 p-3">
                    <div className="flex flex-wrap items-center justify-between gap-2">
                      <span className="text-xs font-semibold text-neutral-800 dark:text-neutral-100">
                        {registration.tunnel_name || registration.tunnel_uuid}
                      </span>
                      <div className="flex gap-1">
                        <SettingsBadge variant="neutral">{registration.target_kind}</SettingsBadge>
                        <SettingsBadge variant="neutral">{registration.skill_id}</SettingsBadge>
                      </div>
                    </div>
                    <div className="mt-1 text-[11px] text-neutral-500 dark:text-neutral-400 font-mono break-all">
                      {backendUrl
                        ? tunnelsApi.ingressUrl(backendUrl, registration.tunnel_uuid)
                        : t('webhooks.resolvingBackendUrl')}
                    </div>
                  </div>
                ))}
              </div>
            )}
          </div>
        </SettingsSection>

        {/* Captured Requests */}
        <SettingsSection title={t('webhooks.capturedRequests')}>
          <div className="px-4 py-3 space-y-2">
            {logs.length === 0 ? (
              <SettingsEmptyState label={t('webhooks.noRequestsCaptured')} />
            ) : (
              <div className="space-y-2">
                {logs.map(entry => (
                  <button
                    key={entry.correlation_id}
                    type="button"
                    onClick={() => setSelectedCorrelationId(entry.correlation_id)}
                    className={`w-full rounded-xl border p-3 text-left transition-colors ${
                      selectedLog?.correlation_id === entry.correlation_id
                        ? 'border-primary-300 dark:border-primary-500/40 bg-primary-50 dark:bg-primary-500/10'
                        : 'border-neutral-200 dark:border-neutral-800 bg-neutral-50 dark:bg-neutral-800/60 hover:bg-neutral-100 dark:hover:bg-neutral-800'
                    }`}>
                    <div className="flex items-center justify-between gap-2">
                      <span className="text-xs font-semibold text-neutral-800 dark:text-neutral-100">
                        {entry.method} {entry.path}
                      </span>
                      <span className="text-[10px] text-neutral-500 dark:text-neutral-400">
                        {entry.status_code ?? '...'}
                      </span>
                    </div>
                    <div className="mt-1 text-[11px] text-neutral-500 dark:text-neutral-400">
                      {entry.tunnel_name}{' '}
                      {entry.skill_id ? `· ${entry.skill_id}` : `· ${t('webhooks.unrouted')}`} ·{' '}
                      {formatDateTime(entry.updated_at)}
                    </div>
                  </button>
                ))}

                {selectedLog && (
                  <div className="rounded-xl border border-neutral-200 dark:border-neutral-800 bg-neutral-50 dark:bg-neutral-800/60 p-3 space-y-3">
                    <div>
                      <div className="text-xs font-semibold text-neutral-800 dark:text-neutral-100">
                        {selectedLog.method} {selectedLog.path}
                      </div>
                      <div className="text-[10px] text-neutral-500 dark:text-neutral-400 font-mono">
                        {selectedLog.correlation_id}
                      </div>
                    </div>

                    <div className="flex flex-wrap gap-1">
                      <SettingsBadge variant="neutral">{selectedLog.stage}</SettingsBadge>
                      <SettingsBadge variant="neutral">
                        {selectedLog.status_code ?? t('webhooks.pending')}
                      </SettingsBadge>
                      <SettingsBadge variant="neutral">
                        {selectedLog.skill_id || t('webhooks.unrouted')}
                      </SettingsBadge>
                    </div>

                    {selectedLog.error_message && (
                      <SettingsStatusLine
                        saving={false}
                        error={selectedLog.error_message}
                        savingLabel=""
                      />
                    )}

                    <PayloadBlock
                      title={t('webhooks.requestHeaders')}
                      value={prettyJson(selectedLog.request_headers)}
                    />
                    <PayloadBlock
                      title={t('webhooks.queryParams')}
                      value={prettyJson(selectedLog.request_query)}
                    />
                    <PayloadBlock
                      title={t('webhooks.requestBody')}
                      value={decodeBase64Preview(selectedLog.request_body) || t('webhooks.empty')}
                    />
                    <PayloadBlock
                      title={t('webhooks.responseHeaders')}
                      value={prettyJson(selectedLog.response_headers)}
                    />
                    <PayloadBlock
                      title={t('webhooks.responseBody')}
                      value={decodeBase64Preview(selectedLog.response_body) || t('webhooks.empty')}
                    />
                    {selectedLog.raw_payload != null && (
                      <PayloadBlock
                        title={t('webhooks.rawPayload')}
                        value={prettyJson(selectedLog.raw_payload)}
                      />
                    )}
                  </div>
                )}
              </div>
            )}
          </div>
        </SettingsSection>
      </div>
    </PanelPage>
  );
};

function PayloadBlock({ title, value }: { title: string; value: string }) {
  return (
    <details className="text-xs">
      <summary className="cursor-pointer font-semibold text-stone-500 dark:text-neutral-400 uppercase tracking-wide text-[10px]">
        {title}
      </summary>
      <pre className="mt-1 max-h-40 overflow-auto rounded-lg border border-stone-200 dark:border-neutral-800 bg-stone-950 dark:bg-neutral-50 p-2 text-[11px] text-stone-100 whitespace-pre-wrap break-words">
        {value}
      </pre>
    </details>
  );
}

export default WebhooksDebugPanel;
