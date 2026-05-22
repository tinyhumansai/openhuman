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
 */
import debug from 'debug';
import { useEffect } from 'react';

import {
  setChannelConnectionStatus,
  upsertChannelConnection,
} from '../store/channelConnectionsSlice';
import { useAppDispatch } from '../store/hooks';
import type { ChannelAuthMode, ChannelType } from '../types/channels';

const log = debug('channels:oauth-listener');

// Module-level constant so the default identity is stable across renders.
// Without this, an inline default array literal would land in the effect's
// dep array and re-subscribe the global oauth:* listeners on every parent
// render. (CodeRabbit on PR #2256.)
const DEFAULT_OAUTH_CAPABILITIES = ['read', 'write'] as const;

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

  useEffect(() => {
    const channelKey = channel.toLowerCase();

    const handleSuccess = (event: Event) => {
      const detail = (event as CustomEvent<OAuthSuccessDetail>).detail;
      const toolkit = detail?.toolkit?.toLowerCase();
      if (!toolkit || toolkit !== channelKey) return;

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

      const lastError =
        detail?.message ||
        'OAuth sign-in did not complete. Try again and approve access to continue.';
      log('oauth error for channel=%s authMode=%s code=%s', channel, authMode, detail?.errorCode);
      dispatch(setChannelConnectionStatus({ channel, authMode, status: 'error', lastError }));
    };

    window.addEventListener('oauth:success', handleSuccess);
    window.addEventListener('oauth:error', handleError);
    return () => {
      window.removeEventListener('oauth:success', handleSuccess);
      window.removeEventListener('oauth:error', handleError);
    };
  }, [dispatch, channel, authMode, capabilitiesOnSuccess]);
}
