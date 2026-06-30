import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import { useOAuthConnectionListener } from '../../hooks/useOAuthConnectionListener';
import { useT } from '../../lib/i18n/I18nContext';
import { useCoreState } from '../../providers/CoreStateProvider';
import { channelConnectionsApi } from '../../services/api/channelConnectionsApi';
import { callCoreRpc } from '../../services/coreRpcClient';
import {
  clearOtherPendingForChannel,
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
import { isLocalSessionToken } from '../../utils/localSession';
import { openUrl } from '../../utils/openUrl';
import { restartCoreProcess } from '../../utils/tauriCommands/core';
import {
  ChannelAuthFields,
  ChannelAuthModeCard,
  ChannelConfigError,
  ChannelConnectActions,
  useChannelAuthFormState,
} from './channelConfigPrimitives';

const log = debug('channels:telegram');

interface TelegramConfigProps {
  definition: ChannelDefinition;
}

const TelegramConfig = ({ definition }: TelegramConfigProps) => {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const channelConnections = useAppSelector(state => state.channelConnections);
  const { snapshot } = useCoreState();
  const isLocalSession = isLocalSessionToken(snapshot.sessionToken);
  const visibleAuthModes = definition.auth_modes.filter(
    spec => !isLocalSession || (spec.mode !== 'managed_dm' && spec.mode !== 'oauth')
  );

  const MANAGED_DM_CONNECTING_MESSAGE = t('channels.telegram.managedDmConnecting');
  const MANAGED_DM_TIMEOUT_MESSAGE = t('channels.telegram.managedDmTimeout');

  const [clearMemoryOnDisconnect, setClearMemoryOnDisconnect] = useState<Record<string, boolean>>(
    {}
  );
  const { busyKeys, fieldValues, error, setError, runBusy, updateField } =
    useChannelAuthFormState();
  const managedDmPollControllers = useRef<Record<string, AbortController>>({});

  const stopManagedDmPolling = useCallback((key: string) => {
    managedDmPollControllers.current[key]?.abort();
    delete managedDmPollControllers.current[key];
  }, []);

  useEffect(() => {
    return () => {
      for (const controller of Object.values(managedDmPollControllers.current)) {
        controller.abort();
      }
      managedDmPollControllers.current = {};
    };
  }, []);

  // Bridge OAuth deep-link completions into Redux. Previously absent on the
  // Telegram panel, so OAuth attempts that succeeded in the browser would
  // never clear the `connecting` badge here. Fixes the Telegram half of
  // #2128 and inherits the shared error-transition behavior.
  useOAuthConnectionListener({ channel: 'telegram', authMode: 'oauth' });

  const startManagedDmPolling = useCallback(
    (key: string, linkToken: string) => {
      stopManagedDmPolling(key);
      const controller = new AbortController();
      managedDmPollControllers.current[key] = controller;

      const POLL_INTERVAL_MS = 3_000;
      const POLL_TIMEOUT_MS = 5 * 60 * 1_000;

      void (async () => {
        log('polling telegram link status via core RPC', { key, tokenLength: linkToken.length });
        const startedAt = Date.now();

        try {
          while (Date.now() - startedAt < POLL_TIMEOUT_MS) {
            if (controller.signal.aborted) return;

            try {
              const check = await channelConnectionsApi.telegramLoginCheck(linkToken);
              if (check.linked) {
                log('telegram managed dm linked via core RPC', { key, details: check.details });
                dispatch(
                  upsertChannelConnection({
                    channel: 'telegram',
                    authMode: 'managed_dm',
                    patch: { status: 'connected', lastError: undefined, capabilities: ['dm'] },
                  })
                );
                return;
              }
            } catch {
              // Best-effort polling: keep trying until timeout or cancellation.
            }

            await new Promise<void>(resolve => {
              const timer = window.setTimeout(resolve, POLL_INTERVAL_MS);
              const onAbort = () => {
                window.clearTimeout(timer);
                resolve();
              };
              controller.signal.addEventListener('abort', onAbort, { once: true });
            });
          }

          if (controller.signal.aborted) return;

          dispatch(
            upsertChannelConnection({
              channel: 'telegram',
              authMode: 'managed_dm',
              patch: { status: 'error', lastError: MANAGED_DM_TIMEOUT_MESSAGE },
            })
          );
          setError(MANAGED_DM_TIMEOUT_MESSAGE);
        } catch (pollError) {
          if (controller.signal.aborted) return;

          const msg = pollError instanceof Error ? pollError.message : String(pollError);
          log('managed dm polling failed', { key, error: msg });
          dispatch(
            upsertChannelConnection({
              channel: 'telegram',
              authMode: 'managed_dm',
              patch: { status: 'error', lastError: msg },
            })
          );
          setError(msg);
        } finally {
          if (managedDmPollControllers.current[key] === controller) {
            delete managedDmPollControllers.current[key];
          }
        }
      })();
    },
    [dispatch, setError, stopManagedDmPolling, MANAGED_DM_TIMEOUT_MESSAGE]
  );

  const handleConnect = useCallback(
    (spec: AuthModeSpec) => {
      const key = `telegram:${spec.mode}`;
      void runBusy(key, async () => {
        // Abort sibling managed-dm polls before clearing their slice rows;
        // a still-running poll could otherwise complete after the clear and
        // dispatch the sibling back to connected/error, leaking the prior
        // attempt into state. (CodeRabbit on PR #2256.) Only managed_dm
        // polls today, so stop that one explicitly.
        const managedDmKey = 'telegram:managed_dm';
        if (key !== managedDmKey) stopManagedDmPolling(managedDmKey);

        // Cancel any sibling auth mode still mid-`connecting` so the panel
        // doesn't pin multiple methods simultaneously (#2128).
        dispatch(clearOtherPendingForChannel({ channel: 'telegram', exceptAuthMode: spec.mode }));
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
                lastError: t('channels.fieldRequired', '{field} is required').replace(
                  '{field}',
                  t(`channels.telegram.fields.${field.key}.label`, field.label || field.key)
                ),
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
            try {
              const loginStart = await channelConnectionsApi.telegramLoginStart();
              log('telegram login start success', {
                key,
                tokenLength: loginStart.linkToken.length,
                botUsername: loginStart.botUsername,
              });
              await openUrl(loginStart.telegramUrl);
              dispatch(
                upsertChannelConnection({
                  channel: 'telegram',
                  authMode: spec.mode,
                  patch: { status: 'connecting', lastError: MANAGED_DM_CONNECTING_MESSAGE },
                })
              );
              startManagedDmPolling(key, loginStart.linkToken);
            } catch (loginStartError) {
              const msg =
                loginStartError instanceof Error
                  ? loginStartError.message
                  : String(loginStartError);
              log('telegram login start failed', { key, error: msg });
              dispatch(
                upsertChannelConnection({
                  channel: 'telegram',
                  authMode: spec.mode,
                  patch: { status: 'error', lastError: msg },
                })
              );
              setError(msg);
            }
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
        if (result.restart_required) {
          log('restart required after connect — restarting core process');
          try {
            await restartCoreProcess();
            log('core process restarted successfully');
            dispatch(
              upsertChannelConnection({
                channel: 'telegram',
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
            setError(t('channels.telegram.savedRestartRequired'));
          }
        } else {
          dispatch(
            upsertChannelConnection({
              channel: 'telegram',
              authMode: spec.mode,
              patch: { status: 'connected', lastError: undefined, capabilities: ['read', 'write'] },
            })
          );
        }
      });
    },
    [
      dispatch,
      fieldValues,
      runBusy,
      startManagedDmPolling,
      stopManagedDmPolling,
      MANAGED_DM_CONNECTING_MESSAGE,
      setError,
      t,
    ]
  );

  const handleDisconnect = useCallback(
    (authMode: ChannelAuthMode) => {
      const key = `telegram:${authMode}`;
      void runBusy(key, async () => {
        log('disconnecting telegram via %s', authMode);
        stopManagedDmPolling(`telegram:${authMode}`);
        await channelConnectionsApi.disconnectChannel('telegram', authMode, {
          clearMemory: Boolean(clearMemoryOnDisconnect[key]),
        });
        setClearMemoryOnDisconnect(prev => ({ ...prev, [key]: false }));
        dispatch(disconnectChannelConnection({ channel: 'telegram', authMode }));
      });
    },
    [clearMemoryOnDisconnect, dispatch, runBusy, stopManagedDmPolling]
  );

  return (
    <div className="space-y-3">
      <div className="rounded-lg border border-primary-200 dark:border-primary-500/30 bg-primary-50/80 dark:bg-primary-500/10 px-4 py-3 text-sm text-content-secondary">
        <p className="font-medium text-content">{t('channels.telegram.remoteControlTitle')}</p>
        <p className="mt-1 text-xs text-content-secondary">
          {t('channels.telegram.remoteControlBody')}
        </p>
      </div>

      {error && <ChannelConfigError message={error} />}

      {isLocalSession && visibleAuthModes.length !== definition.auth_modes.length && (
        <div className="rounded-lg border border-line bg-surface-muted px-4 py-3 text-sm text-content-secondary">
          {t('channels.localManagedUnavailable')}
        </div>
      )}

      {visibleAuthModes.map(spec => {
        const compositeKey = `telegram:${spec.mode}`;
        const connection = channelConnections.connections.telegram?.[spec.mode];
        const status: ChannelConnectionStatus = connection?.status ?? 'disconnected';

        return (
          <ChannelAuthModeCard
            key={spec.mode}
            title={t(`channels.authMode.${spec.mode}`)}
            description={t(`channels.telegram.authMode.${spec.mode}.description`)}
            status={status}
            lastError={connection?.lastError}>
            {spec.fields.length > 0 && (
              <ChannelAuthFields
                spec={spec}
                compositeKey={compositeKey}
                fieldValues={fieldValues}
                onChange={updateField}
                disabled={busyKeys[compositeKey]}
                mapField={field => ({
                  ...field,
                  label: t(`channels.telegram.fields.${field.key}.label`, field.label),
                  placeholder: field.placeholder
                    ? t(`channels.telegram.fields.${field.key}.placeholder`, field.placeholder)
                    : field.placeholder,
                })}
              />
            )}

            {status === 'connected' && (
              <label className="mt-3 flex items-start gap-2 rounded-lg border border-line bg-surface px-3 py-2">
                <input
                  type="checkbox"
                  checked={Boolean(clearMemoryOnDisconnect[compositeKey])}
                  onChange={event =>
                    setClearMemoryOnDisconnect(prev => ({
                      ...prev,
                      [compositeKey]: event.currentTarget.checked,
                    }))
                  }
                  className="mt-0.5 h-4 w-4 rounded border-line-strong text-primary-600 focus:ring-primary-500"
                />
                <span className="min-w-0">
                  <span className="block text-xs font-medium text-content">
                    {t('accounts.disconnectClearMemory')}
                  </span>
                  <span className="block text-[11px] text-content-muted">
                    {t('accounts.disconnectClearMemoryHint')}
                  </span>
                </span>
              </label>
            )}

            <ChannelConnectActions
              busy={busyKeys[compositeKey]}
              status={status}
              connectLabel={
                status === 'connected'
                  ? t('channels.telegram.reconnect')
                  : t('channels.telegram.connect')
              }
              disconnectLabel={t('accounts.disconnect')}
              onConnect={() => handleConnect(spec)}
              onDisconnect={() => handleDisconnect(spec.mode)}
              showConnect
            />
          </ChannelAuthModeCard>
        );
      })}
    </div>
  );
};

export default TelegramConfig;
