import { useCallback, useMemo, useState } from 'react';

import ChannelCapabilities from '../../channels/ChannelCapabilities';
import ChannelFieldInput from '../../channels/ChannelFieldInput';
import ChannelStatusBadge from '../../channels/ChannelStatusBadge';
import { AUTH_MODE_LABELS } from '../../../lib/channels/definitions';
import { resolvePreferredAuthModeForChannel } from '../../../lib/channels/routing';
import { useChannelDefinitions } from '../../../hooks/useChannelDefinitions';
import { channelConnectionsApi } from '../../../services/api/channelConnectionsApi';
import { callCoreRpc } from '../../../services/coreRpcClient';
import {
  disconnectChannelConnection,
  setChannelConnectionStatus,
  setDefaultMessagingChannel,
  upsertChannelConnection,
} from '../../../store/channelConnectionsSlice';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';
import type {
  AuthModeSpec,
  ChannelAuthMode,
  ChannelConnectionStatus,
  ChannelType,
} from '../../../types/channels';
import { openUrl } from '../../../utils/openUrl';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const MessagingPanel = () => {
  const { navigateBack } = useSettingsNavigation();
  const dispatch = useAppDispatch();
  const channelConnections = useAppSelector(state => state.channelConnections);
  const { definitions, loading, error: loadError } = useChannelDefinitions();

  const [error, setError] = useState<string | null>(null);
  const [busyKeys, setBusyKeys] = useState<Record<string, boolean>>({});
  const [fieldValues, setFieldValues] = useState<Record<string, Record<string, string>>>({});

  const recommendedRoute = useMemo(() => {
    const channel = channelConnections.defaultMessagingChannel;
    const authMode = resolvePreferredAuthModeForChannel(channelConnections, channel);
    return authMode ? `${channel} via ${authMode}` : 'No active route';
  }, [channelConnections]);

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

  const handleSetDefaultChannel = useCallback(
    (channel: ChannelType) => {
      const key = `default:${channel}`;
      void runBusy(key, async () => {
        dispatch(setDefaultMessagingChannel(channel));
        await channelConnectionsApi.updatePreferences(channel);
      });
    },
    [dispatch, runBusy]
  );

  const updateField = useCallback((compositeKey: string, fieldKey: string, value: string) => {
    setFieldValues(prev => ({
      ...prev,
      [compositeKey]: { ...(prev[compositeKey] ?? {}), [fieldKey]: value },
    }));
  }, []);

  const handleConnect = useCallback(
    (channel: ChannelType, spec: AuthModeSpec) => {
      const key = `${channel}:${spec.mode}`;
      void runBusy(key, async () => {
        dispatch(
          setChannelConnectionStatus({ channel, authMode: spec.mode, status: 'connecting' })
        );

        const credentials: Record<string, string> = {};
        for (const field of spec.fields) {
          const val = fieldValues[key]?.[field.key]?.trim() ?? '';
          if (field.required && !val) {
            dispatch(
              setChannelConnectionStatus({
                channel,
                authMode: spec.mode,
                status: 'error',
                lastError: `${field.label} is required`,
              })
            );
            return;
          }
          if (val) credentials[field.key] = val;
        }

        const result = await channelConnectionsApi.connectChannel(channel, {
          authMode: spec.mode,
          credentials: Object.keys(credentials).length > 0 ? credentials : undefined,
        });

        if (result.status === 'pending_auth' && result.auth_action) {
          dispatch(
            upsertChannelConnection({
              channel,
              authMode: spec.mode,
              patch: {
                status: 'connecting',
                lastError: result.message ?? `Initiate ${result.auth_action} flow`,
              },
            })
          );

          if (result.auth_action.includes('oauth')) {
            try {
              const oauthResponse = await callCoreRpc<{ result: { oauthUrl?: string } }>({
                method: 'openhuman.auth.oauth_connect',
                params: { provider: channel, skillId: channel },
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

        dispatch(
          upsertChannelConnection({
            channel,
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
    (channel: ChannelType, authMode: ChannelAuthMode) => {
      const key = `${channel}:${authMode}`;
      void runBusy(key, async () => {
        await channelConnectionsApi.disconnectChannel(channel, authMode);
        dispatch(disconnectChannelConnection({ channel, authMode }));
      });
    },
    [dispatch, runBusy]
  );

  const displayError = error || loadError;

  return (
    <div className="overflow-hidden h-full flex flex-col">
      <SettingsHeader title="Messaging" showBackButton={true} onBack={navigateBack} />

      <div className="flex-1 overflow-y-auto p-4 space-y-4">
        {/* Default channel selector */}
        <section className="rounded-xl border border-stone-800/60 bg-black/40 p-4 space-y-3">
          <h3 className="text-sm font-semibold text-white">Default Messaging Channel</h3>
          <div className="grid grid-cols-2 gap-2">
            {definitions.map(def => {
              const channelId = def.id as ChannelType;
              const selected = channelConnections.defaultMessagingChannel === channelId;
              const busyKey = `default:${channelId}`;
              return (
                <button
                  key={channelId}
                  type="button"
                  onClick={() => handleSetDefaultChannel(channelId)}
                  disabled={busyKeys[busyKey]}
                  className={`rounded-lg border px-3 py-2 text-sm transition-colors ${
                    selected
                      ? 'border-primary-500/60 bg-primary-500/20 text-primary-200'
                      : 'border-stone-700 bg-stone-900/30 text-stone-300 hover:border-stone-500'
                  }`}>
                  {def.display_name}
                </button>
              );
            })}
          </div>
          <p className="text-xs text-stone-400">
            Active route: <span className="text-primary-300">{recommendedRoute}</span>
          </p>
        </section>

        {displayError && (
          <div className="rounded-lg border border-coral-500/40 bg-coral-500/10 px-4 py-3 text-sm text-coral-100">
            {displayError}
          </div>
        )}

        {loading && (
          <div className="rounded-xl border border-stone-800/60 bg-black/40 p-4 text-sm text-stone-400">
            Loading channel definitions...
          </div>
        )}

        {!loading &&
          definitions.map(def => {
            const channelId = def.id as ChannelType;
            return (
              <section
                key={channelId}
                className="rounded-xl border border-stone-800/60 bg-black/40 p-4">
                <div className="mb-4">
                  <h3 className="text-base font-semibold text-white">{def.display_name}</h3>
                  <p className="text-xs text-stone-400">{def.description}</p>
                  <ChannelCapabilities capabilities={def.capabilities} />
                </div>

                <div className="space-y-3">
                  {def.auth_modes.map(spec => {
                    const compositeKey = `${channelId}:${spec.mode}`;
                    const connection = channelConnections.connections[channelId]?.[spec.mode];
                    const status: ChannelConnectionStatus = connection?.status ?? 'disconnected';

                    return (
                      <div
                        key={spec.mode}
                        className="rounded-lg border border-stone-800 bg-stone-900/20 p-3">
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
                              />
                            ))}
                          </div>
                        )}

                        <div className="mt-3 flex gap-2">
                          <button
                            type="button"
                            disabled={busyKeys[compositeKey]}
                            onClick={() => handleConnect(channelId, spec)}
                            className="rounded-lg bg-primary-500 px-3 py-1.5 text-xs font-medium text-white hover:bg-primary-600 disabled:opacity-50">
                            {status === 'connected' ? 'Reconnect' : 'Connect'}
                          </button>
                          <button
                            type="button"
                            disabled={busyKeys[compositeKey] || status === 'disconnected'}
                            onClick={() => handleDisconnect(channelId, spec.mode)}
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
          })}
      </div>
    </div>
  );
};

export default MessagingPanel;
