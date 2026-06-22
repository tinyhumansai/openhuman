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
import DiscordServerChannelPicker from './DiscordServerChannelPicker';

const log = debug('channels:discord');
const LINK_TIMEOUT_MS = 5 * 60 * 1_000;
const LINK_POLL_INTERVAL_MS = 3_000;

interface DiscordConfigProps {
  definition: ChannelDefinition;
}

const DiscordConfig = ({ definition }: DiscordConfigProps) => {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const channelConnections = useAppSelector(state => state.channelConnections);
  const { snapshot } = useCoreState();
  const isLocalSession = isLocalSessionToken(snapshot.sessionToken);
  const visibleAuthModes = definition.auth_modes.filter(
    spec => !isLocalSession || (spec.mode !== 'managed_dm' && spec.mode !== 'oauth')
  );

  const [clearMemoryOnDisconnect, setClearMemoryOnDisconnect] = useState<Record<string, boolean>>(
    {}
  );
  const { busyKeys, fieldValues, error, setError, runBusy, updateField } =
    useChannelAuthFormState();
  /** Pending link tokens, keyed by compositeKey (discord:managed_dm). Only present while polling. */
  const [linkToken, setLinkToken] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const pollAbort = useRef<AbortController | null>(null);

  // Stop polling on unmount
  useEffect(() => {
    return () => {
      pollAbort.current?.abort();
    };
  }, []);

  // Centralised OAuth deep-link bridge — also handles `oauth:error` so failed
  // sign-ins transition out of `connecting` instead of pinning the badge. See
  // useOAuthConnectionListener.ts for the per-channel matching contract. Fixes
  // the Discord half of #2128.
  useOAuthConnectionListener({ channel: 'discord', authMode: 'oauth' });

  const startLinkPolling = useCallback(
    (token: string) => {
      pollAbort.current?.abort();
      const controller = new AbortController();
      pollAbort.current = controller;
      const startedAt = Date.now();

      void (async () => {
        while (Date.now() - startedAt < LINK_TIMEOUT_MS) {
          if (controller.signal.aborted) return;

          try {
            const check = await channelConnectionsApi.discordLinkCheck(token);
            if (check.linked) {
              log('discord managed link completed');
              setLinkToken(null);
              dispatch(
                upsertChannelConnection({
                  channel: 'discord',
                  authMode: 'managed_dm',
                  patch: { status: 'connected', lastError: undefined, capabilities: ['dm'] },
                })
              );
              return;
            }
          } catch (err) {
            log('discord link check failed: %o', err);
          }

          await new Promise<void>(resolve => {
            const timer = window.setTimeout(resolve, LINK_POLL_INTERVAL_MS);
            controller.signal.addEventListener(
              'abort',
              () => {
                window.clearTimeout(timer);
                resolve();
              },
              { once: true }
            );
          });
        }

        if (controller.signal.aborted) return;

        setLinkToken(null);
        dispatch(
          upsertChannelConnection({
            channel: 'discord',
            authMode: 'managed_dm',
            patch: { status: 'error', lastError: t('channels.discord.linkTokenExpired') },
          })
        );
      })();
    },
    [dispatch, t]
  );

  const handleConnect = useCallback(
    (spec: AuthModeSpec) => {
      const key = `discord:${spec.mode}`;
      void runBusy(key, async () => {
        // Cancel any in-flight managed-link poll before clearing sibling
        // state. Without this, a stale poll completion could later dispatch
        // `managed_dm` back to connected/error, reviving a flow the user
        // just switched away from. (CodeRabbit on PR #2256.)
        pollAbort.current?.abort();
        setLinkToken(null);

        // Drop any sibling auth mode that's still mid-`connecting` so the
        // panel doesn't show two methods pinned simultaneously (#2128).
        dispatch(clearOtherPendingForChannel({ channel: 'discord', exceptAuthMode: spec.mode }));
        dispatch(
          setChannelConnectionStatus({
            channel: 'discord',
            authMode: spec.mode,
            status: 'connecting',
          })
        );
        log('connecting discord via %s', spec.mode);

        const credentials: Record<string, string> = {};
        for (const field of spec.fields) {
          // `rawVal` is `undefined` only when the user never touched the field;
          // an empty string means they entered something and then cleared it.
          const rawVal = fieldValues[key]?.[field.key];
          const val = rawVal?.trim() ?? '';
          if (field.required && !val) {
            dispatch(
              setChannelConnectionStatus({
                channel: 'discord',
                authMode: spec.mode,
                status: 'error',
                lastError: t('channels.fieldRequired', '{field} is required').replace(
                  '{field}',
                  t(`channels.discord.fields.${field.key}.label`, field.label || field.key)
                ),
              })
            );
            return;
          }
          if (val) {
            credentials[field.key] = val;
          } else if (rawVal !== undefined) {
            // Field was edited and then cleared — submit an explicit empty value
            // instead of omitting it, so the backend can distinguish "cleared"
            // from "never entered". For the allowlist this is what makes clearing
            // it on reconnect mean "allow everyone" rather than silently reusing
            // the previously-saved list (#3794 review — Codex P2).
            credentials[field.key] = '';
          }
        }

        const result = await channelConnectionsApi.connectChannel('discord', {
          authMode: spec.mode,
          credentials: Object.keys(credentials).length > 0 ? credentials : undefined,
        });
        log('connect result: %o', result);

        if (result.status === 'pending_auth' && result.auth_action) {
          if (result.auth_action === 'discord_managed_link') {
            const linkStart = await channelConnectionsApi.discordLinkStart();
            log('discord link token issued, length=%d', linkStart.linkToken.length);
            setLinkToken(linkStart.linkToken);
            dispatch(
              upsertChannelConnection({
                channel: 'discord',
                authMode: spec.mode,
                patch: { status: 'connecting', lastError: undefined },
              })
            );
            startLinkPolling(linkStart.linkToken);
          } else if (result.auth_action.includes('oauth')) {
            dispatch(
              upsertChannelConnection({
                channel: 'discord',
                authMode: spec.mode,
                patch: { status: 'connecting', lastError: undefined },
              })
            );
            try {
              const oauthResponse = await callCoreRpc<{ result: { oauthUrl?: string } }>({
                method: 'openhuman.auth.oauth_connect',
                params: { provider: 'discord', skillId: 'discord' },
              });
              if (oauthResponse.result?.oauthUrl) {
                await openUrl(oauthResponse.result.oauthUrl);
              }
            } catch {
              // best-effort
            }
          }
          return;
        }

        if (result.restart_required) {
          try {
            await restartCoreProcess();
            dispatch(
              upsertChannelConnection({
                channel: 'discord',
                authMode: spec.mode,
                patch: {
                  status: 'connected',
                  lastError: undefined,
                  capabilities: ['read', 'write'],
                },
              })
            );
          } catch {
            setError(t('channels.discord.savedRestartRequired'));
          }
        } else {
          dispatch(
            upsertChannelConnection({
              channel: 'discord',
              authMode: spec.mode,
              patch: { status: 'connected', lastError: undefined, capabilities: ['read', 'write'] },
            })
          );
        }
      });
    },
    [dispatch, fieldValues, runBusy, setError, startLinkPolling, t]
  );

  const handleDisconnect = useCallback(
    (authMode: ChannelAuthMode) => {
      const key = `discord:${authMode}`;
      void runBusy(`discord:${authMode}`, async () => {
        log('disconnecting discord via %s', authMode);
        pollAbort.current?.abort();
        setLinkToken(null);
        await channelConnectionsApi.disconnectChannel('discord', authMode, {
          clearMemory: Boolean(clearMemoryOnDisconnect[key]),
        });
        setClearMemoryOnDisconnect(prev => ({ ...prev, [key]: false }));
        dispatch(disconnectChannelConnection({ channel: 'discord', authMode }));
      });
    },
    [clearMemoryOnDisconnect, dispatch, runBusy]
  );

  const copyToken = useCallback(() => {
    if (!linkToken) return;
    void navigator.clipboard.writeText(linkToken).then(() => {
      setCopied(true);
      window.setTimeout(() => setCopied(false), 2000);
    });
  }, [linkToken]);

  return (
    <div className="space-y-3">
      {error && <ChannelConfigError message={error} />}

      {isLocalSession && visibleAuthModes.length !== definition.auth_modes.length && (
        <div className="rounded-lg border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 px-4 py-3 text-sm text-stone-700 dark:text-neutral-200">
          {t('channels.localManagedUnavailable')}
        </div>
      )}

      {visibleAuthModes.map(spec => {
        const compositeKey = `discord:${spec.mode}`;
        const connection = channelConnections.connections.discord?.[spec.mode];
        const status: ChannelConnectionStatus = connection?.status ?? 'disconnected';
        const busy = busyKeys[compositeKey] ?? false;

        return (
          <ChannelAuthModeCard
            key={spec.mode}
            title={t(`channels.authMode.${spec.mode}`)}
            description={t(`channels.discord.authMode.${spec.mode}.description`)}
            status={status}
            lastError={connection?.lastError}>
            {/* Field inputs — only for non-managed modes */}
            {spec.fields.length > 0 && status !== 'connected' && (
              <ChannelAuthFields
                spec={spec}
                compositeKey={compositeKey}
                fieldValues={fieldValues}
                onChange={updateField}
                disabled={busy}
                mapField={field => ({
                  ...field,
                  label: t(`channels.discord.fields.${field.key}.label`, field.label),
                  placeholder: field.placeholder
                    ? t(`channels.discord.fields.${field.key}.placeholder`, field.placeholder)
                    : field.placeholder,
                })}
              />
            )}

            {/* Token card — managed_dm connecting state */}
            {spec.mode === 'managed_dm' && linkToken && status === 'connecting' && (
              <div className="mt-3 rounded-lg border border-primary-200 dark:border-primary-500/30 bg-primary-50/60 dark:bg-primary-500/15 p-3 space-y-2">
                <p className="text-xs font-medium text-primary-700 dark:text-primary-300">
                  {t('channels.discord.linkTokenLabel')}
                </p>
                <div className="flex items-center gap-2">
                  <code className="flex-1 rounded bg-white dark:bg-neutral-900 border border-primary-200 dark:border-primary-500/30 px-2 py-1 text-xs font-mono text-stone-800 dark:text-neutral-100 select-all break-all">
                    {linkToken}
                  </code>
                  <button
                    type="button"
                    onClick={copyToken}
                    className="shrink-0 rounded-lg border border-primary-300 dark:border-primary-500/40 px-2 py-1 text-xs font-medium text-primary-700 dark:text-primary-300 hover:bg-primary-100 dark:hover:bg-primary-500/20">
                    {copied ? t('common.copied') : t('common.copy')}
                  </button>
                </div>
                <p className="text-xs text-stone-500 dark:text-neutral-400">
                  {t('channels.discord.linkTokenInstruction').replace('{token}', linkToken)}
                </p>
                <p className="text-xs text-amber-600 font-medium">
                  {t('channels.discord.linkTokenOnce')}
                </p>
              </div>
            )}

            {/* Connected state for managed_dm — show only Disconnect */}
            {spec.mode === 'managed_dm' && status === 'connected' ? (
              <>
                <label className="mt-3 flex items-start gap-2 rounded-lg border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-3 py-2">
                  <input
                    type="checkbox"
                    checked={Boolean(clearMemoryOnDisconnect[compositeKey])}
                    onChange={event =>
                      setClearMemoryOnDisconnect(prev => ({
                        ...prev,
                        [compositeKey]: event.currentTarget.checked,
                      }))
                    }
                    className="mt-0.5 h-4 w-4 rounded border-stone-300 text-primary-600 focus:ring-primary-500"
                  />
                  <span className="min-w-0">
                    <span className="block text-xs font-medium text-stone-800 dark:text-neutral-100">
                      {t('accounts.disconnectClearMemory')}
                    </span>
                    <span className="block text-[11px] text-stone-500 dark:text-neutral-400">
                      {t('accounts.disconnectClearMemoryHint')}
                    </span>
                  </span>
                </label>
                <div className="mt-3 flex items-center justify-between">
                  <p className="text-xs text-sage-700 dark:text-sage-300 font-medium">
                    {t('channels.discord.accountLinked')}
                  </p>
                  <ChannelConnectActions
                    busy={busy}
                    status={status}
                    connectLabel={t('channels.discord.connect')}
                    disconnectLabel={t('accounts.disconnect')}
                    onDisconnect={() => handleDisconnect(spec.mode)}
                    showConnect={false}
                    className="mt-0"
                  />
                </div>
              </>
            ) : /* Connect / Disconnect buttons for all other modes and states */
            spec.mode !== 'managed_dm' || status !== 'connecting' ? (
              <>
                {status === 'connected' && (
                  <label className="mt-3 flex items-start gap-2 rounded-lg border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-3 py-2">
                    <input
                      type="checkbox"
                      checked={Boolean(clearMemoryOnDisconnect[compositeKey])}
                      onChange={event =>
                        setClearMemoryOnDisconnect(prev => ({
                          ...prev,
                          [compositeKey]: event.currentTarget.checked,
                        }))
                      }
                      className="mt-0.5 h-4 w-4 rounded border-stone-300 text-primary-600 focus:ring-primary-500"
                    />
                    <span className="min-w-0">
                      <span className="block text-xs font-medium text-stone-800 dark:text-neutral-100">
                        {t('accounts.disconnectClearMemory')}
                      </span>
                      <span className="block text-[11px] text-stone-500 dark:text-neutral-400">
                        {t('accounts.disconnectClearMemoryHint')}
                      </span>
                    </span>
                  </label>
                )}
                <ChannelConnectActions
                  busy={busy}
                  status={status}
                  connectLabel={t('channels.discord.connect')}
                  disconnectLabel={t('accounts.disconnect')}
                  onConnect={() => handleConnect(spec)}
                  onDisconnect={() => handleDisconnect(spec.mode)}
                />
              </>
            ) : null}

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
          </ChannelAuthModeCard>
        );
      })}
    </div>
  );
};

export default DiscordConfig;
