import debug from 'debug';
import { useCallback, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { channelConnectionsApi } from '../../services/api/channelConnectionsApi';
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
  ChannelType,
} from '../../types/channels';
import { restartCoreProcess } from '../../utils/tauriCommands/core';
import ChannelFieldInput from './ChannelFieldInput';
import ChannelStatusBadge from './ChannelStatusBadge';

const log = debug('channels:credential');

interface CredentialChannelConfigProps {
  definition: ChannelDefinition;
}

/**
 * Generic credential ("API key") connect form for channels whose auth is a set
 * of text/secret/boolean fields declared by the core (e.g. Lark/Feishu,
 * DingTalk). Renders the field schema straight from the definition, collects
 * credentials, and drives the standard `channels_connect` / `channels_disconnect`
 * RPCs — these channels persist to TOML and require a core restart to activate.
 *
 * Field labels/placeholders prefer a per-channel i18n key but fall back to the
 * core-provided label, so no per-locale keys are required for a new channel.
 */
const CredentialChannelConfig = ({ definition }: CredentialChannelConfigProps) => {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const channel = definition.id as ChannelType;
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
      setError(e instanceof Error ? e.message : String(e));
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
      const compositeKey = `${channel}:${spec.mode}`;
      void runBusy(compositeKey, async () => {
        dispatch(
          setChannelConnectionStatus({ channel, authMode: spec.mode, status: 'connecting' })
        );
        log('connecting %s via %s', channel, spec.mode);

        const credentials: Record<string, string> = {};
        for (const field of spec.fields) {
          const raw = fieldValues[compositeKey]?.[field.key] ?? '';
          const val = field.field_type === 'boolean' ? raw : raw.trim();
          // Booleans are always semantically set (checkbox is on or off), so an
          // untouched required boolean must not fail the empty-value check.
          if (field.required && field.field_type !== 'boolean' && !val) {
            const label = t(`channels.${channel}.fields.${field.key}.label`, field.label);
            dispatch(
              setChannelConnectionStatus({
                channel,
                authMode: spec.mode,
                status: 'error',
                lastError: t('channels.fieldRequired', '{field} is required').replace(
                  '{field}',
                  label
                ),
              })
            );
            return;
          }
          if (val) credentials[field.key] = val;
        }

        let result;
        try {
          result = await channelConnectionsApi.connectChannel(channel, {
            authMode: spec.mode,
            credentials: Object.keys(credentials).length > 0 ? credentials : undefined,
          });
        } catch (e) {
          // Surface the failure on the connection itself so the badge leaves
          // `connecting` — runBusy only updates the local banner otherwise.
          dispatch(
            setChannelConnectionStatus({
              channel,
              authMode: spec.mode,
              status: 'error',
              lastError: e instanceof Error ? e.message : String(e),
            })
          );
          throw e;
        }
        log('connect result: %o', result);

        if (result.restart_required) {
          try {
            await restartCoreProcess();
          } catch {
            // Credentials were saved but the core didn't restart, so the channel
            // is not live yet — don't mark it connected; reflect the pending state.
            dispatch(
              setChannelConnectionStatus({
                channel,
                authMode: spec.mode,
                status: 'error',
                lastError: t(
                  'channels.savedRestartRequired',
                  'Channel saved. Restart the app to activate it.'
                ),
              })
            );
            return;
          }
        }
        dispatch(
          upsertChannelConnection({
            channel,
            authMode: spec.mode,
            patch: { status: 'connected', lastError: undefined, capabilities: ['read', 'write'] },
          })
        );
      });
    },
    [channel, dispatch, fieldValues, runBusy, t]
  );

  const handleDisconnect = useCallback(
    (authMode: ChannelAuthMode) => {
      void runBusy(`${channel}:${authMode}`, async () => {
        log('disconnecting %s via %s', channel, authMode);
        await channelConnectionsApi.disconnectChannel(channel, authMode);
        dispatch(disconnectChannelConnection({ channel, authMode }));
      });
    },
    [channel, dispatch, runBusy]
  );

  return (
    <div className="space-y-3">
      {error && (
        <div className="rounded-lg border border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 px-4 py-3 text-sm text-coral-700 dark:text-coral-300">
          {error}
        </div>
      )}

      {definition.auth_modes.map(spec => {
        const compositeKey = `${channel}:${spec.mode}`;
        const connection = channelConnections.connections[channel]?.[spec.mode];
        const status: ChannelConnectionStatus = connection?.status ?? 'disconnected';
        const busy = busyKeys[compositeKey] ?? false;

        return (
          <div
            key={spec.mode}
            className="rounded-lg border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 p-3">
            <div className="flex items-start justify-between gap-3">
              <p className="text-xs text-stone-500 dark:text-neutral-400">{spec.description}</p>
              <ChannelStatusBadge status={status} />
            </div>

            {connection?.lastError && (
              <p className="text-xs text-coral-600 mt-2">{connection.lastError}</p>
            )}

            {spec.fields.length > 0 && status !== 'connected' && (
              <div className="mt-3 space-y-2">
                {spec.fields.map(field => (
                  <ChannelFieldInput
                    key={field.key}
                    field={{
                      ...field,
                      label: t(`channels.${channel}.fields.${field.key}.label`, field.label),
                      placeholder: field.placeholder
                        ? t(
                            `channels.${channel}.fields.${field.key}.placeholder`,
                            field.placeholder
                          )
                        : field.placeholder,
                    }}
                    value={fieldValues[compositeKey]?.[field.key] ?? ''}
                    onChange={val => updateField(compositeKey, field.key, val)}
                    disabled={busy}
                  />
                ))}
              </div>
            )}

            <div className="mt-3 flex gap-2">
              {status !== 'connected' && (
                <button
                  type="button"
                  disabled={busy}
                  onClick={() => handleConnect(spec)}
                  className="rounded-lg bg-primary-500 px-3 py-1.5 text-xs font-medium text-white hover:bg-primary-600 disabled:opacity-50">
                  {t('channels.connect', 'Connect')}
                </button>
              )}
              <button
                type="button"
                disabled={busy || status === 'disconnected'}
                onClick={() => handleDisconnect(spec.mode)}
                className="rounded-lg border border-stone-200 dark:border-neutral-800 px-3 py-1.5 text-xs font-medium text-stone-600 dark:text-neutral-300 hover:border-stone-300 dark:hover:border-neutral-700 disabled:opacity-50">
                {t('accounts.disconnect')}
              </button>
            </div>
          </div>
        );
      })}
    </div>
  );
};

export default CredentialChannelConfig;
