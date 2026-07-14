/**
 * OAuth Connection Listener Hook
 *
 * Bridges the global `oauth:success` / `oauth:error` deep-link CustomEvents
 * (dispatched from `utils/desktopDeepLinkListener.ts`) into the
 * `channelConnections` Redux slice so that the right channel/authMode badge
 * transitions out of `connecting` when the OAuth flow finishes in the system
 * browser.
 *
 * Per-channel config panels (`DiscordConfig`, `TelegramConfig`, …) call this
 * hook with their channel + the auth mode that owns the OAuth path. Each panel
 * used to roll its own effect, which is how #2128 happened: `DiscordConfig`
 * had a success listener, `TelegramConfig` had none, neither handled errors,
 * so failed or completed OAuth flows could leave the badge pinned at
 * `Connecting` forever.
 *
 * Centralising this means new channels with OAuth auth modes inherit correct
 * pending-state transitions for free.
 *
 * It also recovers from the *no deep link ever arrives* case (#4299): when
 * Discord rejects the redirect_uri, the user cancels, or the browser lands on
 * an error page, neither `oauth:success` nor `oauth:error` fires, so the badge
 * would otherwise stay pinned at `connecting` forever. While a channel sits at
 * `connecting` this hook watches for the user returning to the app (window
 * focus / tab visible) and an absolute timeout, then flips to a retryable
 * `error` if no OAuth event resolved it.
 */
import debug from 'debug';
import { useEffect, useRef } from 'react';

import { useT } from '../lib/i18n/I18nContext';
import {
  setChannelConnectionStatus,
  upsertChannelConnection,
} from '../store/channelConnectionsSlice';
import { getDeepLinkAuthState } from '../store/deepLinkAuthState';
import { useAppDispatch, useAppSelector } from '../store/hooks';
import type { ChannelAuthMode, ChannelType } from '../types/channels';

const log = debug('channels:oauth-listener');

// Module-level constant so the default identity is stable across renders.
// Without this, an inline default array literal would land in the effect's
// dep array and re-subscribe the global oauth:* listeners on every parent
// render. (CodeRabbit on PR #2256.)
const DEFAULT_OAUTH_CAPABILITIES = ['read', 'write'] as const;

// How long a channel may sit at `connecting` with no OAuth deep link before we
// give up and surface a retryable error. Covers the "user never came back to
// the app" path (closed the browser tab on Discord's redirect_uri error page,
// switched away permanently). Mirrors `OAuthProviderButton`'s 300s login
// fallback but shorter — a channel connect that hasn't round-tripped in 2min is
// not coming back. (#4299)
const OAUTH_CONNECTING_TIMEOUT_MS = 120_000;

// Grace window applied after a recovery trigger (window focus / tab visible /
// absolute timeout) before flipping `connecting` → `error`. A *successful*
// OAuth deep link arrives shortly after the OS refocuses the app, but its
// dispatch is delayed by `focusMainWindow()` + the async app-version gate in
// `desktopDeepLinkListener.handleOAuthDeepLink`. Waiting out this grace lets a
// real success/error event win the race (and cancel the timer) so we never
// flash a false error on the happy path. (#4299)
const OAUTH_RECOVER_GRACE_MS = 2_500;

interface OAuthSuccessDetail {
  integrationId?: string;
  toolkit?: string;
}

interface OAuthErrorDetail {
  provider?: string;
  errorCode?: string;
  message?: string;
}

export interface UseOAuthConnectionListenerOptions {
  /** Channel that owns the OAuth flow (e.g. 'discord', 'telegram'). */
  channel: ChannelType;
  /** Auth mode that the OAuth deep-link should resolve to. */
  authMode: ChannelAuthMode;
  /**
   * Capabilities to record on the connection when OAuth succeeds. Mirrors the
   * existing per-channel defaults; kept explicit so each call site stays
   * self-documenting.
   */
  capabilitiesOnSuccess?: readonly string[];
}

/**
 * Subscribe to OAuth completion / failure deep-link events for one channel.
 *
 * Match key: the event's `toolkit` (success) or `provider` (error) field is
 * compared case-insensitively to `channel`. Events for other channels are
 * ignored so multiple panels can mount the hook simultaneously without
 * stepping on each other.
 */
export function useOAuthConnectionListener({
  channel,
  authMode,
  capabilitiesOnSuccess = DEFAULT_OAUTH_CAPABILITIES,
}: UseOAuthConnectionListenerOptions): void {
  const dispatch = useAppDispatch();
  const { t } = useT();

  // Current status for this channel/authMode. Drives whether recovery watchers
  // are armed (only while `connecting`). `statusRef` mirrors it so the deferred
  // grace callback reads the freshest value without itself being an effect dep.
  const status = useAppSelector(
    state => state.channelConnections.connections[channel]?.[authMode]?.status
  );
  const statusRef = useRef(status);
  statusRef.current = status;

  useEffect(() => {
    const channelKey = channel.toLowerCase();

    // Shared timers so a real oauth:success / oauth:error can cancel a pending
    // recovery, guaranteeing the deep link always wins the race.
    let graceTimer: ReturnType<typeof setTimeout> | undefined;
    let absoluteTimer: ReturnType<typeof setTimeout> | undefined;
    const clearTimers = () => {
      if (graceTimer !== undefined) clearTimeout(graceTimer);
      if (absoluteTimer !== undefined) clearTimeout(absoluteTimer);
      graceTimer = undefined;
      absoluteTimer = undefined;
    };

    const handleSuccess = (event: Event) => {
      const detail = (event as CustomEvent<OAuthSuccessDetail>).detail;
      const toolkit = detail?.toolkit?.toLowerCase();
      if (!toolkit || toolkit !== channelKey) return;

      clearTimers();
      log('oauth success for channel=%s authMode=%s', channel, authMode);
      dispatch(
        upsertChannelConnection({
          channel,
          authMode,
          patch: {
            status: 'connected',
            lastError: undefined,
            capabilities: [...capabilitiesOnSuccess],
          },
        })
      );
    };

    const handleError = (event: Event) => {
      const detail = (event as CustomEvent<OAuthErrorDetail>).detail;
      const provider = detail?.provider?.toLowerCase();
      if (!provider || provider !== channelKey) return;

      clearTimers();
      const lastError =
        detail?.message ||
        'OAuth sign-in did not complete. Try again and approve access to continue.';
      log('oauth error for channel=%s authMode=%s code=%s', channel, authMode, detail?.errorCode);
      dispatch(setChannelConnectionStatus({ channel, authMode, status: 'error', lastError }));
    };

    window.addEventListener('oauth:success', handleSuccess);
    window.addEventListener('oauth:error', handleError);

    // Recovery is only relevant while the badge is pinned at `connecting`.
    if (status !== 'connecting') {
      return () => {
        window.removeEventListener('oauth:success', handleSuccess);
        window.removeEventListener('oauth:error', handleError);
        clearTimers();
      };
    }

    // Flip to a retryable error, but only if the flow is *still* unresolved:
    // a deep link landing during the grace window updates the store and makes
    // this a no-op. Skip while a deep-link auth round-trip is in flight (the
    // callback flips `isProcessing` before dispatching its event, same guard as
    // `OAuthProviderButton`).
    const recover = (label: string) => {
      if (statusRef.current !== 'connecting') return;
      if (getDeepLinkAuthState().isProcessing) {
        log('[%s] recover via %s skipped (deep-link processing)', channel, label);
        return;
      }
      const channelLabel = channel.charAt(0).toUpperCase() + channel.slice(1);
      log('[%s] recover via %s -> error (no oauth deep link arrived)', channel, label);
      dispatch(
        setChannelConnectionStatus({
          channel,
          authMode,
          status: 'error',
          lastError: t(
            'channels.oauth.connectRecoverFailed',
            "Couldn't finish connecting {channel}. The sign-in window closed before access was granted — try again."
          ).replace('{channel}', channelLabel),
        })
      );
    };

    const scheduleRecover = (label: string) => {
      if (graceTimer !== undefined) clearTimeout(graceTimer);
      graceTimer = setTimeout(() => recover(label), OAUTH_RECOVER_GRACE_MS);
    };

    // User returned to the app from the system browser without a deep link.
    const handleFocus = () => scheduleRecover('focus');
    const handleVisibility = () => {
      if (document.visibilityState === 'visible') scheduleRecover('visibilitychange');
    };

    window.addEventListener('focus', handleFocus);
    document.addEventListener('visibilitychange', handleVisibility);
    // Backstop for the user who never refocuses the app at all.
    absoluteTimer = setTimeout(() => scheduleRecover('timeout'), OAUTH_CONNECTING_TIMEOUT_MS);

    return () => {
      window.removeEventListener('oauth:success', handleSuccess);
      window.removeEventListener('oauth:error', handleError);
      window.removeEventListener('focus', handleFocus);
      document.removeEventListener('visibilitychange', handleVisibility);
      clearTimers();
    };
  }, [dispatch, channel, authMode, capabilitiesOnSuccess, status, t]);
}
