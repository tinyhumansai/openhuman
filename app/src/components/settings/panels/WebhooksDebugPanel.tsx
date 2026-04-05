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
import { PrimaryButton } from './components/ActionPanel';
import SectionCard from './components/SectionCard';

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

      <div className="p-4 space-y-4">
        <SectionCard
          title="Overview"
          priority="development"
          icon={
            <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M8 9l4-4 4 4m0 6l-4 4-4-4M3 12h18"
              />
            </svg>
          }>
          <div className="flex flex-wrap items-center gap-3">
            <PrimaryButton onClick={() => void loadData()} loading={loading} variant="secondary">
              Refresh
            </PrimaryButton>
            <PrimaryButton
              onClick={() => void handleClearLogs()}
              loading={clearing}
              disabled={logs.length === 0}
              variant="outline">
              Clear Logs
            </PrimaryButton>
            <div className="rounded-lg border border-stone-200 bg-white px-3 py-2 text-sm text-stone-700">
              Registered:{' '}
              <span className="font-semibold text-stone-900">{registrations.length}</span>
            </div>
            <div className="rounded-lg border border-stone-200 bg-white px-3 py-2 text-sm text-stone-700">
              Captured: <span className="font-semibold text-stone-900">{logs.length}</span>
            </div>
            <div className="rounded-lg border border-stone-200 bg-white px-3 py-2 text-sm text-stone-700">
              Live:{' '}
              <span
                className={isLive ? 'font-semibold text-sage-700' : 'font-semibold text-stone-500'}>
                {isLive ? 'connected' : 'disconnected'}
              </span>
            </div>
          </div>

          {lastEvent && (
            <div className="rounded-lg border border-stone-200 bg-white px-3 py-2 text-xs text-stone-600">
              Last event: <span className="font-medium text-stone-900">{lastEvent.event_type}</span>{' '}
              at {formatDateTime(lastEvent.timestamp)}
            </div>
          )}

          {error && (
            <div className="rounded-lg border border-coral-500/40 bg-coral-50 px-3 py-2 text-sm text-coral-700">
              {error}
            </div>
          )}
        </SectionCard>

        <SectionCard
          title="Registered Webhooks"
          priority="tools"
          icon={
            <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M13.828 10.172a4 4 0 010 5.656l-2 2a4 4 0 01-5.656-5.656l1-1m5-5a4 4 0 015.656 5.656l-1 1m-5 5l5-5"
              />
            </svg>
          }>
          {registrations.length === 0 ? (
            <div className="rounded-lg border border-dashed border-stone-300 bg-white px-4 py-6 text-sm text-stone-500">
              No webhook registrations are active in the runtime.
            </div>
          ) : (
            <div className="space-y-3">
              {registrations.map(registration => (
                <div
                  key={registration.tunnel_uuid}
                  className="rounded-xl border border-stone-200 bg-white p-4">
                  <div className="flex flex-wrap items-center justify-between gap-2">
                    <div>
                      <div className="text-sm font-semibold text-stone-900">
                        {registration.tunnel_name || registration.tunnel_uuid}
                      </div>
                      <div className="text-xs text-stone-500">{registration.tunnel_uuid}</div>
                    </div>
                    <div className="flex flex-wrap gap-2 text-xs">
                      <span className="rounded-full bg-stone-100 px-3 py-1 font-medium text-stone-700">
                        {registration.target_kind}
                      </span>
                      <span className="rounded-full bg-stone-100 px-3 py-1 font-medium text-stone-700">
                        {registration.skill_id}
                      </span>
                    </div>
                  </div>
                  <div className="mt-3 grid gap-2 text-xs text-stone-600">
                    <div>
                      Target URL:{' '}
                      <span className="font-mono text-stone-900">
                        {tunnelsApi.ingressUrl(fallbackBackendUrl, registration.tunnel_uuid)}
                      </span>
                    </div>
                    {registration.backend_tunnel_id && (
                      <div>
                        Backend tunnel ID:{' '}
                        <span className="font-mono text-stone-900">
                          {registration.backend_tunnel_id}
                        </span>
                      </div>
                    )}
                  </div>
                </div>
              ))}
            </div>
          )}
        </SectionCard>

        <SectionCard
          title="Captured Requests"
          priority="development"
          icon={
            <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"
              />
            </svg>
          }>
          {logs.length === 0 ? (
            <div className="rounded-lg border border-dashed border-stone-300 bg-white px-4 py-6 text-sm text-stone-500">
              No webhook requests captured yet.
            </div>
          ) : (
            <div className="grid gap-4 lg:grid-cols-[minmax(0,0.95fr)_minmax(0,1.25fr)]">
              <div className="space-y-2">
                {logs.map(entry => (
                  <button
                    key={entry.correlation_id}
                    type="button"
                    onClick={() => setSelectedCorrelationId(entry.correlation_id)}
                    className={`w-full rounded-xl border p-3 text-left transition-colors ${
                      selectedLog?.correlation_id === entry.correlation_id
                        ? 'border-primary-300 bg-primary-50'
                        : 'border-stone-200 bg-white hover:bg-stone-50'
                    }`}>
                    <div className="flex items-center justify-between gap-2">
                      <div className="text-sm font-semibold text-stone-900">
                        {entry.method} {entry.path}
                      </div>
                      <div className="text-xs font-medium text-stone-500">
                        {entry.status_code ?? '...'}
                      </div>
                    </div>
                    <div className="mt-1 text-xs text-stone-500">
                      {entry.tunnel_name} {entry.skill_id ? `• ${entry.skill_id}` : '• unrouted'}
                    </div>
                    <div className="mt-1 text-xs text-stone-500">
                      {formatDateTime(entry.updated_at)}
                    </div>
                  </button>
                ))}
              </div>

              {selectedLog && (
                <div className="rounded-xl border border-stone-200 bg-white p-4 space-y-4">
                  <div className="flex flex-wrap items-start justify-between gap-3">
                    <div>
                      <div className="text-lg font-semibold text-stone-900">
                        {selectedLog.method} {selectedLog.path}
                      </div>
                      <div className="text-xs text-stone-500">{selectedLog.correlation_id}</div>
                    </div>
                    <div className="flex flex-wrap gap-2 text-xs">
                      <span className="rounded-full bg-stone-100 px-3 py-1 font-medium text-stone-700">
                        stage: {selectedLog.stage}
                      </span>
                      <span className="rounded-full bg-stone-100 px-3 py-1 font-medium text-stone-700">
                        status: {selectedLog.status_code ?? 'pending'}
                      </span>
                    </div>
                  </div>

                  <div className="grid gap-2 text-sm text-stone-700">
                    <div>
                      Tunnel:{' '}
                      <span className="font-medium text-stone-900">{selectedLog.tunnel_name}</span>
                    </div>
                    <div>
                      Tunnel UUID:{' '}
                      <span className="font-mono text-xs text-stone-900">
                        {selectedLog.tunnel_uuid}
                      </span>
                    </div>
                    <div>
                      Skill:{' '}
                      <span className="font-medium text-stone-900">
                        {selectedLog.skill_id || 'unrouted'}
                      </span>
                    </div>
                    <div>Received: {formatDateTime(selectedLog.timestamp)}</div>
                    <div>Updated: {formatDateTime(selectedLog.updated_at)}</div>
                  </div>

                  {selectedLog.error_message && (
                    <div className="rounded-lg border border-coral-500/40 bg-coral-50 px-3 py-2 text-sm text-coral-700">
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
        </SectionCard>
      </div>
    </div>
  );
};

function PayloadBlock({ title, value }: { title: string; value: string }) {
  return (
    <div>
      <div className="mb-1 text-xs font-semibold uppercase tracking-wide text-stone-500">
        {title}
      </div>
      <pre className="max-h-56 overflow-auto rounded-lg border border-stone-200 bg-stone-950 p-3 text-xs text-stone-100 whitespace-pre-wrap break-words">
        {value}
      </pre>
    </div>
  );
}

export default WebhooksDebugPanel;
