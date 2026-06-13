import debug from 'debug';
import { useCallback } from 'react';

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
import {
  ChannelAuthFields,
  ChannelAuthModeCard,
  ChannelConfigError,
  ChannelConnectActions,
  useChannelAuthFormState,
} from './channelConfigPrimitives';

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

  const { busyKeys, fieldValues, error, runBusy, updateField } = useChannelAuthFormState();

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
      {error && <ChannelConfigError message={error} />}

      {definition.auth_modes.map(spec => {
        const compositeKey = `${channel}:${spec.mode}`;
        const connection = channelConnections.connections[channel]?.[spec.mode];
        const status: ChannelConnectionStatus = connection?.status ?? 'disconnected';
        const busy = busyKeys[compositeKey] ?? false;

        return (
          <ChannelAuthModeCard
            key={spec.mode}
            description={spec.description}
            status={status}
            lastError={connection?.lastError}>
            {spec.fields.length > 0 && status !== 'connected' && (
              <ChannelAuthFields
                spec={spec}
                compositeKey={compositeKey}
                fieldValues={fieldValues}
                onChange={updateField}
                disabled={busy}
                mapField={field => ({
                  ...field,
                  label: t(`channels.${channel}.fields.${field.key}.label`, field.label),
                  placeholder: field.placeholder
                    ? t(`channels.${channel}.fields.${field.key}.placeholder`, field.placeholder)
                    : field.placeholder,
                })}
              />
            )}

            <ChannelConnectActions
              busy={busy}
              status={status}
              connectLabel={t('channels.connect', 'Connect')}
              disconnectLabel={t('accounts.disconnect')}
              onConnect={() => handleConnect(spec)}
              onDisconnect={() => handleDisconnect(spec.mode)}
            />
          </ChannelAuthModeCard>
        );
      })}
    </div>
  );
};

export default CredentialChannelConfig;
