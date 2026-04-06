import { useCallback, useEffect, useMemo, useState } from 'react';

import { tunnelsApi } from '../../../services/api/tunnelsApi';
import { getCoreHttpBaseUrl } from '../../../services/coreRpcClient';
import { BACKEND_URL } from '../../../utils/config';
import {
  openhumanWebhooksClearLogs,
  openhumanWebhooksListLogs,
  openhumanWebhooksListRegistrations,
  type WebhookDebugEvent,
  type WebhookDebugLogEntry,
  type WebhookDebugRegistration,
} from '../../../utils/tauriCommands';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const LOG_LIMIT = 100;

const fallbackBackendUrl = BACKEND_URL || 'https://api.tinyhumans.ai';

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
  const { navigateBack } = useSettingsNavigation();
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
        loadError instanceof Error ? loadError.message : 'Failed to load webhook debug data'
      );
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadData();
  }, [loadData]);

  useEffect(() => {
    let eventSource: EventSource | null = null;
    let cancelled = false;

    const connect = async () => {
      try {
        const baseUrl = await getCoreHttpBaseUrl();
        if (cancelled) return;
        eventSource = new EventSource(`${baseUrl}/events/webhooks`);

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
    const confirmed = window.confirm('Clear all captured webhook debug logs?');
    if (!confirmed) return;

    setClearing(true);
    setError(null);
    try {
      await openhumanWebhooksClearLogs();
      await loadData();
    } catch (clearError) {
      setError(clearError instanceof Error ? clearError.message : 'Failed to clear webhook logs');
    } finally {
      setClearing(false);
    }
  }, [loadData]);

  return (
    <div>
      <SettingsHeader title="Webhooks Debug" showBackButton={true} onBack={navigateBack} />

      <div className="p-4 space-y-5">
        {/* Status bar */}
        <div className="flex flex-wrap items-center gap-2 text-xs">
          <button
            type="button"
            onClick={() => void loadData()}
            disabled={loading}
            className="rounded-lg border border-stone-200 bg-stone-50 px-3 py-1.5 font-medium text-stone-700 hover:bg-stone-100 disabled:opacity-50">
            {loading ? 'Loading...' : 'Refresh'}
          </button>
          <button
            type="button"
            onClick={() => void handleClearLogs()}
            disabled={clearing || logs.length === 0}
            className="rounded-lg border border-stone-200 bg-stone-50 px-3 py-1.5 font-medium text-stone-700 hover:bg-stone-100 disabled:opacity-50">
            {clearing ? 'Clearing...' : 'Clear Logs'}
          </button>
          <span className="text-stone-500">
            {registrations.length} registered &middot; {logs.length} captured &middot;{' '}
            <span className={isLive ? 'text-sage-600' : 'text-stone-400'}>
              {isLive ? 'live' : 'disconnected'}
            </span>
          </span>
        </div>

        {error && (
          <div className="rounded-lg border border-coral-200 bg-coral-50 px-3 py-2 text-xs text-coral-700">
            {error}
          </div>
        )}

        {lastEvent && (
          <div className="text-xs text-stone-500">
            Last event: <span className="font-medium text-stone-700">{lastEvent.event_type}</span>{' '}
            at {formatDateTime(lastEvent.timestamp)}
          </div>
        )}

        {/* Registrations */}
        <section className="space-y-2">
          <h3 className="text-sm font-semibold text-stone-900">Registered Webhooks</h3>
          {registrations.length === 0 ? (
            <p className="text-xs text-stone-400">No active registrations.</p>
          ) : (
            <div className="space-y-2">
              {registrations.map(registration => (
                <div
                  key={registration.tunnel_uuid}
                  className="rounded-xl border border-stone-200 bg-stone-50 p-3">
                  <div className="flex flex-wrap items-center justify-between gap-2">
                    <span className="text-xs font-semibold text-stone-900">
                      {registration.tunnel_name || registration.tunnel_uuid}
                    </span>
                    <div className="flex gap-1 text-[10px]">
                      <span className="rounded-full bg-stone-200 px-2 py-0.5 text-stone-600">
                        {registration.target_kind}
                      </span>
                      <span className="rounded-full bg-stone-200 px-2 py-0.5 text-stone-600">
                        {registration.skill_id}
                      </span>
                    </div>
                  </div>
                  <div className="mt-1 text-[11px] text-stone-500 font-mono break-all">
                    {tunnelsApi.ingressUrl(fallbackBackendUrl, registration.tunnel_uuid)}
                  </div>
                </div>
              ))}
            </div>
          )}
        </section>

        {/* Captured Requests */}
        <section className="space-y-2">
          <h3 className="text-sm font-semibold text-stone-900">Captured Requests</h3>
          {logs.length === 0 ? (
            <p className="text-xs text-stone-400">No webhook requests captured yet.</p>
          ) : (
            <div className="space-y-2">
              {logs.map(entry => (
                <button
                  key={entry.correlation_id}
                  type="button"
                  onClick={() => setSelectedCorrelationId(entry.correlation_id)}
                  className={`w-full rounded-xl border p-3 text-left transition-colors ${
                    selectedLog?.correlation_id === entry.correlation_id
                      ? 'border-primary-300 bg-primary-50'
                      : 'border-stone-200 bg-stone-50 hover:bg-stone-100'
                  }`}>
                  <div className="flex items-center justify-between gap-2">
                    <span className="text-xs font-semibold text-stone-900">
                      {entry.method} {entry.path}
                    </span>
                    <span className="text-[10px] text-stone-500">{entry.status_code ?? '...'}</span>
                  </div>
                  <div className="mt-1 text-[11px] text-stone-500">
                    {entry.tunnel_name} {entry.skill_id ? `· ${entry.skill_id}` : '· unrouted'} ·{' '}
                    {formatDateTime(entry.updated_at)}
                  </div>
                </button>
              ))}

              {selectedLog && (
                <div className="rounded-xl border border-stone-200 bg-stone-50 p-3 space-y-3">
                  <div>
                    <div className="text-xs font-semibold text-stone-900">
                      {selectedLog.method} {selectedLog.path}
                    </div>
                    <div className="text-[10px] text-stone-400 font-mono">
                      {selectedLog.correlation_id}
                    </div>
                  </div>

                  <div className="flex flex-wrap gap-1 text-[10px]">
                    <span className="rounded-full bg-stone-200 px-2 py-0.5 text-stone-600">
                      {selectedLog.stage}
                    </span>
                    <span className="rounded-full bg-stone-200 px-2 py-0.5 text-stone-600">
                      {selectedLog.status_code ?? 'pending'}
                    </span>
                    <span className="rounded-full bg-stone-200 px-2 py-0.5 text-stone-600">
                      {selectedLog.skill_id || 'unrouted'}
                    </span>
                  </div>

                  {selectedLog.error_message && (
                    <div className="rounded-lg border border-coral-200 bg-coral-50 px-3 py-2 text-xs text-coral-700">
                      {selectedLog.error_message}
                    </div>
                  )}

                  <PayloadBlock
                    title="Request Headers"
                    value={prettyJson(selectedLog.request_headers)}
                  />
                  <PayloadBlock
                    title="Query Params"
                    value={prettyJson(selectedLog.request_query)}
                  />
                  <PayloadBlock
                    title="Request Body"
                    value={decodeBase64Preview(selectedLog.request_body) || '[empty]'}
                  />
                  <PayloadBlock
                    title="Response Headers"
                    value={prettyJson(selectedLog.response_headers)}
                  />
                  <PayloadBlock
                    title="Response Body"
                    value={decodeBase64Preview(selectedLog.response_body) || '[empty]'}
                  />
                  {selectedLog.raw_payload != null && (
                    <PayloadBlock title="Raw Payload" value={prettyJson(selectedLog.raw_payload)} />
                  )}
                </div>
              )}
            </div>
          )}
        </section>
      </div>
    </div>
  );
};

function PayloadBlock({ title, value }: { title: string; value: string }) {
  return (
    <details className="text-xs">
      <summary className="cursor-pointer font-semibold text-stone-500 uppercase tracking-wide text-[10px]">
        {title}
      </summary>
      <pre className="mt-1 max-h-40 overflow-auto rounded-lg border border-stone-200 bg-stone-950 p-2 text-[11px] text-stone-100 whitespace-pre-wrap break-words">
        {value}
      </pre>
    </details>
  );
}

export default WebhooksDebugPanel;
