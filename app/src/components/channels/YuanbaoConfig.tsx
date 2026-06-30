import debug from 'debug';
import { useCallback, useEffect, useState } from 'react';

import { AUTH_MODE_LABELS } from '../../lib/channels/definitions';
import { useT } from '../../lib/i18n/I18nContext';
import { channelConnectionsApi } from '../../services/api/channelConnectionsApi';
import {
  disconnectChannelConnection,
  setChannelConnectionStatus,
  upsertChannelConnection,
} from '../../store/channelConnectionsSlice';
import { useAppDispatch, useAppSelector } from '../../store/hooks';
import type { ChannelConnectionStatus, ChannelDefinition } from '../../types/channels';
import { restartCoreProcess } from '../../utils/tauriCommands/core';
import Button from '../ui/Button';
import ChannelFieldInput from './ChannelFieldInput';
import ChannelStatusBadge from './ChannelStatusBadge';

const log = debug('channels:yuanbao');

interface YuanbaoConfigProps {
  definition: ChannelDefinition;
}

const YuanbaoConfig = ({ definition }: YuanbaoConfigProps) => {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const channelConnections = useAppSelector(state => state.channelConnections);

  const [busy, setBusy] = useState(false);
  const [fieldValues, setFieldValues] = useState<Record<string, string>>({
    app_key: '',
    app_secret: '',
  });
  // Per-field inline validation errors, keyed by field.key.
  const [fieldErrors, setFieldErrors] = useState<Record<string, string>>({});

  const updateField = useCallback((fieldKey: string, value: string) => {
    setFieldValues(prev => ({ ...prev, [fieldKey]: value }));
    // Clear the error for this field as the user types.
    setFieldErrors(prev => {
      if (!prev[fieldKey]) return prev;
      const next = { ...prev };
      delete next[fieldKey];
      return next;
    });
  }, []);

  const spec = definition.auth_modes[0];

  // On mount, reset any stale 'connecting' state persisted from a previous session.
  useEffect(() => {
    if (!spec) return;
    const conn = channelConnections.connections.yuanbao?.[spec.mode];
    if (conn?.status === 'connecting') {
      dispatch(
        setChannelConnectionStatus({
          channel: 'yuanbao',
          authMode: spec.mode,
          status: 'disconnected',
        })
      );
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // All useCallback hooks must be called unconditionally.
  const handleConnect = useCallback(() => {
    if (busy) return;
    log('handleConnect entry, spec=%o', spec);
    if (!spec) {
      log('handleConnect aborted — spec is null');
      return;
    }

    const errors: Record<string, string> = {};
    for (const field of spec.fields) {
      const empty = !fieldValues[field.key]?.trim();
      if (field.required && empty) {
        errors[field.key] = t('channels.yuanbao.fieldRequired').replace('{field}', field.label);
      }
    }
    if (Object.keys(errors).length > 0) {
      log('handleConnect validation failed: %o', errors);
      setFieldErrors(errors);
      return;
    }
    log('handleConnect validation passed');

    setFieldErrors({});
    setBusy(true);

    dispatch(
      setChannelConnectionStatus({ channel: 'yuanbao', authMode: spec.mode, status: 'connecting' })
    );

    const credentials: Record<string, string> = {};
    for (const field of spec.fields) {
      const val = fieldValues[field.key]?.trim() ?? '';
      if (val) credentials[field.key] = val;
    }
    log('dispatched connecting, credential keys=%o', Object.keys(credentials));

    void (async () => {
      try {
        log('connecting yuanbao via %s', spec.mode);
        const result = await channelConnectionsApi.connectChannel('yuanbao', {
          authMode: spec.mode,
          credentials,
        });
        log('connect result: %o', result);

        // Only treat explicit "connected" as success. Any other status
        // (e.g. "pending_auth" if a future auth flow gets added) must
        // surface as an error instead of silently dispatching connected.
        if (result.status !== 'connected') {
          const msg = t('channels.yuanbao.unexpectedStatus').replace(
            '{status}',
            result.status ?? ''
          );
          log('unexpected status: %s', result.status);
          dispatch(
            setChannelConnectionStatus({
              channel: 'yuanbao',
              authMode: spec.mode,
              status: 'error',
              lastError: msg,
            })
          );
          return;
        }

        if (result.restart_required) {
          log('restart required after connect — restarting core process');
          try {
            await restartCoreProcess();
            log('core restart complete, dispatching connected');
            dispatch(
              upsertChannelConnection({
                channel: 'yuanbao',
                authMode: spec.mode,
                patch: {
                  status: 'connected',
                  lastError: undefined,
                  capabilities: ['read', 'write'],
                },
              })
            );
          } catch (restartErr) {
            const msg = restartErr instanceof Error ? restartErr.message : String(restartErr);
            log('core restart failed: %s', msg);
            dispatch(
              setChannelConnectionStatus({
                channel: 'yuanbao',
                authMode: spec.mode,
                status: 'error',
                lastError: t('channels.yuanbao.savedRestartRequired'),
              })
            );
          }
        } else {
          log('no restart required, dispatching connected');
          dispatch(
            upsertChannelConnection({
              channel: 'yuanbao',
              authMode: spec.mode,
              patch: { status: 'connected', lastError: undefined, capabilities: ['read', 'write'] },
            })
          );
        }
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e);
        log('handleConnect error: %s', msg);
        dispatch(
          setChannelConnectionStatus({
            channel: 'yuanbao',
            authMode: spec.mode,
            status: 'error',
            lastError: msg,
          })
        );
      } finally {
        setBusy(false);
      }
    })();
  }, [dispatch, fieldValues, spec, t]);

  const handleDisconnect = useCallback(() => {
    if (busy) return;
    if (!spec) return;
    setBusy(true);
    void (async () => {
      try {
        log('disconnecting yuanbao');
        await channelConnectionsApi.disconnectChannel('yuanbao', spec.mode);
        dispatch(disconnectChannelConnection({ channel: 'yuanbao', authMode: spec.mode }));
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e);
        dispatch(
          setChannelConnectionStatus({
            channel: 'yuanbao',
            authMode: spec.mode,
            status: 'error',
            lastError: msg,
          })
        );
      } finally {
        setBusy(false);
      }
    })();
  }, [dispatch, spec]);

  if (!spec) return null;

  const connection = channelConnections.connections.yuanbao?.[spec.mode];
  const status: ChannelConnectionStatus = connection?.status ?? 'disconnected';

  return (
    <div className="space-y-3">
      <div className="rounded-lg border border-line bg-surface-muted p-3">
        <div className="flex items-start justify-between gap-3">
          <div>
            <p className="text-sm font-medium text-content">
              {AUTH_MODE_LABELS[spec.mode] ?? spec.mode}
            </p>
            <p className="text-xs text-content-muted mt-1">{spec.description}</p>
            {connection?.lastError && (
              <p className="text-xs text-coral-600 mt-1">{connection.lastError}</p>
            )}
          </div>
          <ChannelStatusBadge status={status} />
        </div>

        {spec.fields.length > 0 && (
          <div className="mt-3 space-y-2">
            {spec.fields.map(field => {
              return (
                <div key={field.key}>
                  <ChannelFieldInput
                    field={field}
                    value={fieldValues[field.key] ?? ''}
                    onChange={val => updateField(field.key, val)}
                    disabled={busy}
                  />
                  {fieldErrors[field.key] && (
                    <p className="mt-1 text-xs text-coral-600 dark:text-coral-400">
                      {fieldErrors[field.key]}
                    </p>
                  )}
                </div>
              );
            })}
          </div>
        )}

        <div className="mt-3 flex gap-2">
          <Button
            variant="primary"
            size="sm"
            disabled={busy}
            onClick={handleConnect}
            leadingIcon={
              busy ? (
                <svg
                  className="h-3 w-3 animate-spin"
                  xmlns="http://www.w3.org/2000/svg"
                  fill="none"
                  viewBox="0 0 24 24">
                  <circle
                    className="opacity-25"
                    cx="12"
                    cy="12"
                    r="10"
                    stroke="currentColor"
                    strokeWidth="4"
                  />
                  <path
                    className="opacity-75"
                    fill="currentColor"
                    d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
                  />
                </svg>
              ) : undefined
            }>
            {busy
              ? t('channels.yuanbao.connecting')
              : status === 'connected'
                ? t('channels.yuanbao.reconnect')
                : t('channels.yuanbao.connect')}
          </Button>
          <Button
            variant="secondary"
            size="sm"
            disabled={busy || status === 'disconnected'}
            onClick={handleDisconnect}>
            {t('accounts.disconnect')}
          </Button>
        </div>
      </div>
    </div>
  );
};

export default YuanbaoConfig;
