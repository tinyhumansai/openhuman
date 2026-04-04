import debug from 'debug';
import { useCallback, useState } from 'react';

import { AUTH_MODE_LABELS } from '../../lib/channels/definitions';
import { channelConnectionsApi } from '../../services/api/channelConnectionsApi';
import { callCoreRpc } from '../../services/coreRpcClient';
import {
  disconnectChannelConnection,
  setChannelConnectionStatus,
  upsertChannelConnection,
} from '../../store/channelConnectionsSlice';
import { useAppDispatch, useAppSelector } from '../../store/hooks';
import type {
  AuthModeSpec,
  ChannelAuthMode,
  ChannelConnectionStatus,
  ChannelDefinition,
} from '../../types/channels';
import { openUrl } from '../../utils/openUrl';
import ChannelFieldInput from './ChannelFieldInput';
import ChannelStatusBadge from './ChannelStatusBadge';

const log = debug('channels:telegram');
const MANAGED_DM_FOLLOW_UP_MESSAGE = 'Managed DM setup will be enabled in a follow-up update.';

interface TelegramConfigProps {
  definition: ChannelDefinition;
}

const TelegramConfig = ({ definition }: TelegramConfigProps) => {
  const dispatch = useAppDispatch();
  const channelConnections = useAppSelector(state => state.channelConnections);

  const [busyKeys, setBusyKeys] = useState<Record<string, boolean>>({});
  const [fieldValues, setFieldValues] = useState<Record<string, Record<string, string>>>({});
  const [error, setError] = useState<string | null>(null);

  const runBusy = useCallback(async (key: string, task: () => Promise<void>) => {
    setBusyKeys(prev => ({ ...prev, [key]: true }));
    setError(null);
    try {
      await task();
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setError(msg);
    } finally {
      setBusyKeys(prev => ({ ...prev, [key]: false }));
    }
  }, []);

  const updateField = useCallback((compositeKey: string, fieldKey: string, value: string) => {
    setFieldValues(prev => ({
      ...prev,
      [compositeKey]: { ...(prev[compositeKey] ?? {}), [fieldKey]: value },
    }));
  }, []);

  const handleConnect = useCallback(
    (spec: AuthModeSpec) => {
      const key = `telegram:${spec.mode}`;
      void runBusy(key, async () => {
        dispatch(
          setChannelConnectionStatus({
            channel: 'telegram',
            authMode: spec.mode,
            status: 'connecting',
          })
        );
        log('connecting telegram via %s', spec.mode);

        // Build credentials from field values.
        const credentials: Record<string, string> = {};
        for (const field of spec.fields) {
          const val = fieldValues[key]?.[field.key]?.trim() ?? '';
          if (field.required && !val) {
            dispatch(
              setChannelConnectionStatus({
                channel: 'telegram',
                authMode: spec.mode,
                status: 'error',
                lastError: `${field.label} is required`,
              })
            );
            return;
          }
          if (val) credentials[field.key] = val;
        }

        const result = await channelConnectionsApi.connectChannel('telegram', {
          authMode: spec.mode,
          credentials: Object.keys(credentials).length > 0 ? credentials : undefined,
        });
        log('connect result: %o', result);

        if (result.status === 'pending_auth' && result.auth_action) {
          if (result.auth_action === 'telegram_managed_dm') {
            log('managed dm connect requested before backend flow is enabled');
            dispatch(
              upsertChannelConnection({
                channel: 'telegram',
                authMode: spec.mode,
                patch: {
                  status: 'disconnected',
                  lastError: result.message ?? MANAGED_DM_FOLLOW_UP_MESSAGE,
                },
              })
            );
          } else if (result.auth_action.includes('oauth')) {
            dispatch(
              upsertChannelConnection({
                channel: 'telegram',
                authMode: spec.mode,
                patch: { status: 'connecting' },
              })
            );
            try {
              const oauthResponse = await callCoreRpc<{ result: { oauthUrl?: string } }>({
                method: 'openhuman.auth.oauth_connect',
                params: { provider: 'telegram', skillId: 'telegram' },
              });
              if (oauthResponse.result?.oauthUrl) {
                await openUrl(oauthResponse.result.oauthUrl);
              }
            } catch {
              // OAuth URL fetch is best-effort.
            }
          }
          return;
        }

        // Credential-based connection succeeded.
        dispatch(
          upsertChannelConnection({
            channel: 'telegram',
            authMode: spec.mode,
            patch: { status: 'connected', lastError: undefined, capabilities: ['read', 'write'] },
          })
        );

        if (result.restart_required) {
          setError(result.message ?? 'Restart the service to activate the channel.');
        }
      });
    },
    [dispatch, fieldValues, runBusy]
  );

  const handleDisconnect = useCallback(
    (authMode: ChannelAuthMode) => {
      const key = `telegram:${authMode}`;
      void runBusy(key, async () => {
        log('disconnecting telegram via %s', authMode);
        await channelConnectionsApi.disconnectChannel('telegram', authMode);
        dispatch(disconnectChannelConnection({ channel: 'telegram', authMode }));
      });
    },
    [dispatch, runBusy]
  );

  return (
    <section className="rounded-xl border border-stone-800/60 bg-black/40 p-4 space-y-4">
      <div>
        <h3 className="text-base font-semibold text-white">{definition.display_name}</h3>
        <p className="text-xs text-stone-400 mt-1">{definition.description}</p>
      </div>

      {error && (
        <div className="rounded-lg border border-coral-500/40 bg-coral-500/10 px-4 py-3 text-sm text-coral-100">
          {error}
        </div>
      )}

      <div className="space-y-3">
        {definition.auth_modes.map(spec => {
          const compositeKey = `telegram:${spec.mode}`;
          const connection = channelConnections.connections.telegram?.[spec.mode];
          const status: ChannelConnectionStatus = connection?.status ?? 'disconnected';

          return (
            <div key={spec.mode} className="rounded-lg border border-stone-800 bg-stone-900/20 p-3">
              <div className="flex items-start justify-between gap-3">
                <div>
                  <p className="text-sm font-medium text-white">
                    {AUTH_MODE_LABELS[spec.mode] ?? spec.mode}
                  </p>
                  <p className="text-xs text-stone-400 mt-1">{spec.description}</p>
                  {connection?.lastError && (
                    <p className="text-xs text-coral-300 mt-1">{connection.lastError}</p>
                  )}
                </div>
                <ChannelStatusBadge status={status} />
              </div>

              {spec.fields.length > 0 && (
                <div className="mt-3 space-y-2">
                  {spec.fields.map(field => (
                    <ChannelFieldInput
                      key={field.key}
                      field={field}
                      value={fieldValues[compositeKey]?.[field.key] ?? ''}
                      onChange={val => updateField(compositeKey, field.key, val)}
                      disabled={busyKeys[compositeKey]}
                    />
                  ))}
                </div>
              )}

              <div className="mt-3 flex gap-2">
                <button
                  type="button"
                  disabled={busyKeys[compositeKey]}
                  onClick={() => handleConnect(spec)}
                  className="rounded-lg bg-primary-500 px-3 py-1.5 text-xs font-medium text-white hover:bg-primary-600 disabled:opacity-50">
                  {status === 'connected' ? 'Reconnect' : 'Connect'}
                </button>
                <button
                  type="button"
                  disabled={busyKeys[compositeKey] || status === 'disconnected'}
                  onClick={() => handleDisconnect(spec.mode)}
                  className="rounded-lg border border-stone-700 px-3 py-1.5 text-xs font-medium text-stone-300 hover:border-stone-500 disabled:opacity-50">
                  Disconnect
                </button>
              </div>
            </div>
          );
        })}
      </div>
    </section>
  );
};

export default TelegramConfig;
