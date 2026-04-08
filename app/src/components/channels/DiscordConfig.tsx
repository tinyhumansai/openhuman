import debug from 'debug';
import { useCallback, useState } from 'react';

import { AUTH_MODE_LABELS } from '../../lib/channels/definitions';
import { channelConnectionsApi } from '../../services/api/channelConnectionsApi';
import { callCoreRpc } from '../../services/coreRpcClient';
import { restartCoreProcess } from '../../utils/tauriCommands/core';
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
import DiscordServerChannelPicker from './DiscordServerChannelPicker';

const log = debug('channels:discord');

interface DiscordConfigProps {
  definition: ChannelDefinition;
}

const DiscordConfig = ({ definition }: DiscordConfigProps) => {
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
      const key = `discord:${spec.mode}`;
      void runBusy(key, async () => {
        dispatch(
          setChannelConnectionStatus({
            channel: 'discord',
            authMode: spec.mode,
            status: 'connecting',
          })
        );
        log('connecting discord via %s', spec.mode);

        // Build credentials from field values.
        const credentials: Record<string, string> = {};
        for (const field of spec.fields) {
          const val = fieldValues[key]?.[field.key]?.trim() ?? '';
          if (field.required && !val) {
            dispatch(
              setChannelConnectionStatus({
                channel: 'discord',
                authMode: spec.mode,
                status: 'error',
                lastError: `${field.label} is required`,
              })
            );
            return;
          }
          if (val) credentials[field.key] = val;
        }

        const result = await channelConnectionsApi.connectChannel('discord', {
          authMode: spec.mode,
          credentials: Object.keys(credentials).length > 0 ? credentials : undefined,
        });
        log('connect result: %o', result);

        if (result.status === 'pending_auth' && result.auth_action) {
          dispatch(
            upsertChannelConnection({
              channel: 'discord',
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
                params: { provider: 'discord', skillId: 'discord' },
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
            channel: 'discord',
            authMode: spec.mode,
            patch: { status: 'connected', lastError: undefined, capabilities: ['read', 'write'] },
          })
        );

        if (result.restart_required) {
          log('restart required after connect — restarting core process');
          try {
            await restartCoreProcess();
            log('core process restarted successfully');
          } catch (restartErr) {
            const msg = restartErr instanceof Error ? restartErr.message : String(restartErr);
            log('core restart failed: %s', msg);
            setError('Channel saved. Restart the app to activate it.');
          }
        }
      });
    },
    [dispatch, fieldValues, runBusy]
  );

  const handleDisconnect = useCallback(
    (authMode: ChannelAuthMode) => {
      const key = `discord:${authMode}`;
      void runBusy(key, async () => {
        log('disconnecting discord via %s', authMode);
        await channelConnectionsApi.disconnectChannel('discord', authMode);
        dispatch(disconnectChannelConnection({ channel: 'discord', authMode }));
      });
    },
    [dispatch, runBusy]
  );

  return (
    <div className="space-y-3">
      {error && (
        <div className="rounded-lg border border-coral-200 bg-coral-50 px-4 py-3 text-sm text-coral-700">
          {error}
        </div>
      )}

      {definition.auth_modes.map(spec => {
        const compositeKey = `discord:${spec.mode}`;
        const connection = channelConnections.connections.discord?.[spec.mode];
        const status: ChannelConnectionStatus = connection?.status ?? 'disconnected';

        return (
          <div key={spec.mode} className="rounded-lg border border-stone-200 bg-stone-50 p-3">
            <div className="flex items-start justify-between gap-3">
              <div>
                <p className="text-sm font-medium text-stone-900">
                  {AUTH_MODE_LABELS[spec.mode] ?? spec.mode}
                </p>
                <p className="text-xs text-stone-500 mt-1">{spec.description}</p>
                {connection?.lastError && (
                  <p className="text-xs text-coral-600 mt-1">{connection.lastError}</p>
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
                className="rounded-lg border border-stone-200 px-3 py-1.5 text-xs font-medium text-stone-600 hover:border-stone-300 disabled:opacity-50">
                Disconnect
              </button>
            </div>

            {/* Server + Channel picker — shown after successful bot_token connection */}
            {spec.mode === 'bot_token' && status === 'connected' && (
              <DiscordServerChannelPicker
                selectedGuildId={fieldValues[compositeKey]?.guild_id ?? ''}
                selectedChannelId={fieldValues[compositeKey]?.channel_id ?? ''}
                onGuildSelected={guildId => {
                  updateField(compositeKey, 'guild_id', guildId);
                  updateField(compositeKey, 'channel_id', '');
                }}
                onChannelSelected={channelId => updateField(compositeKey, 'channel_id', channelId)}
              />
            )}
          </div>
        );
      })}
    </div>
  );
};

export default DiscordConfig;
