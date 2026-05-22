import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import { useOAuthConnectionListener } from '../../hooks/useOAuthConnectionListener';
import { AUTH_MODE_LABELS } from '../../lib/channels/definitions';
import { useT } from '../../lib/i18n/I18nContext';
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
import { openUrl } from '../../utils/openUrl';
import { restartCoreProcess } from '../../utils/tauriCommands/core';
import ChannelFieldInput from './ChannelFieldInput';
import ChannelStatusBadge from './ChannelStatusBadge';

const log = debug('channels:telegram');

interface TelegramConfigProps {
  definition: ChannelDefinition;
}

const TelegramConfig = ({ definition }: TelegramConfigProps) => {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const channelConnections = useAppSelector(state => state.channelConnections);

  const MANAGED_DM_CONNECTING_MESSAGE = t('channels.telegram.managedDmConnecting');
  const MANAGED_DM_TIMEOUT_MESSAGE = t('channels.telegram.managedDmTimeout');

  const [busyKeys, setBusyKeys] = useState<Record<string, boolean>>({});
  const [fieldValues, setFieldValues] = useState<Record<string, Record<string, string>>>({});
  const [error, setError] = useState<string | null>(null);
  const managedDmPollControllers = useRef<Record<string, AbortController>>({});

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
    [dispatch, stopManagedDmPolling, MANAGED_DM_TIMEOUT_MESSAGE]
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
      t,
    ]
  );

  const handleDisconnect = useCallback(
    (authMode: ChannelAuthMode) => {
      const key = `telegram:${authMode}`;
      void runBusy(key, async () => {
        log('disconnecting telegram via %s', authMode);
        stopManagedDmPolling(`telegram:${authMode}`);
        await channelConnectionsApi.disconnectChannel('telegram', authMode);
        dispatch(disconnectChannelConnection({ channel: 'telegram', authMode }));
      });
    },
    [dispatch, runBusy, stopManagedDmPolling]
  );

  return (
    <div className="space-y-3">
      <div className="rounded-lg border border-primary-200 dark:border-primary-500/30 bg-primary-50/80 dark:bg-primary-500/10 px-4 py-3 text-sm text-stone-700 dark:text-neutral-200">
        <p className="font-medium text-stone-900 dark:text-neutral-100">
          Remote control (Telegram)
        </p>
        <p className="mt-1 text-xs text-stone-600 dark:text-neutral-400">
          From an allowed Telegram chat, send /status, /sessions, /new, or /help. Model routing
          still uses /model and /models.
        </p>
      </div>

      {error && (
        <div className="rounded-lg border border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 px-4 py-3 text-sm text-coral-700 dark:text-coral-300">
          {error}
        </div>
      )}

      {definition.auth_modes.map(spec => {
        const compositeKey = `telegram:${spec.mode}`;
        const connection = channelConnections.connections.telegram?.[spec.mode];
        const status: ChannelConnectionStatus = connection?.status ?? 'disconnected';

        return (
          <div
            key={spec.mode}
            className="rounded-lg border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 p-3">
            <div className="flex items-start justify-between gap-3">
              <div>
                <p className="text-sm font-medium text-stone-900 dark:text-neutral-100">
                  {AUTH_MODE_LABELS[spec.mode] ?? spec.mode}
                </p>
                <p className="text-xs text-stone-500 dark:text-neutral-400 mt-1">
                  {spec.description}
                </p>
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
                {status === 'connected'
                  ? t('channels.telegram.reconnect')
                  : t('channels.telegram.connect')}
              </button>
              <button
                type="button"
                disabled={busyKeys[compositeKey] || status === 'disconnected'}
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

export default TelegramConfig;
