import debug from 'debug';
import { useCallback, useEffect, useState } from 'react';

import { FALLBACK_DEFINITIONS } from '../lib/channels/definitions';
import { channelConnectionsApi } from '../services/api/channelConnectionsApi';
import { store } from '../store';
import {
  completeBreakingMigration,
  setDefaultMessagingChannel,
  upsertChannelConnection,
} from '../store/channelConnectionsSlice';
import { useAppDispatch, useAppSelector } from '../store/hooks';
import {
  type ChannelAuthMode,
  type ChannelConnectionStatus,
  type ChannelDefinition,
  type ChannelStatusEntry,
  isChannelType,
} from '../types/channels';

const log = debug('channels:definitions');

/**
 * Map a backend channel-status entry to the Redux status patch to apply
 * (issue #3712). A connected entry asserts `connected`; a configured-but-failing
 * entry asserts `error` and carries the live failure reason. A not-connected
 * entry with no error is skipped while a connect flow is still `connecting`, so
 * a stale status poll doesn't stomp an in-flight attempt. Returns `null` when
 * there is nothing to assert.
 */
export function resolveStatusPatch(
  entry: ChannelStatusEntry,
  currentStatus: ChannelConnectionStatus | undefined
): { status: ChannelConnectionStatus; lastError: string | undefined } | null {
  if (entry.connected) {
    return { status: 'connected', lastError: undefined };
  }
  if (entry.error) {
    return { status: 'error', lastError: entry.error };
  }
  if (currentStatus === 'connecting') {
    return null;
  }
  return { status: 'disconnected', lastError: undefined };
}

export function useChannelDefinitions() {
  const dispatch = useAppDispatch();
  const channelConnections = useAppSelector(state => state.channelConnections);

  const [definitions, setDefinitions] = useState<ChannelDefinition[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Run breaking migration if needed.
  useEffect(() => {
    if (!channelConnections.migrationCompleted) {
      dispatch(completeBreakingMigration());
    }
  }, [channelConnections.migrationCompleted, dispatch]);

  const loadDefinitions = useCallback(async () => {
    let cancelled = false;
    setLoading(true);
    setError(null);

    try {
      const [defs, statusEntries, defaultChannel] = await Promise.all([
        channelConnectionsApi.listDefinitions().catch(() => null),
        channelConnectionsApi.listStatus().catch(() => null),
        channelConnectionsApi.getDefaultChannel().catch(() => null),
      ]);
      if (cancelled) return;

      // Seed the default messaging channel from the core (source of truth for
      // proactive routing) so the UI reflects the persisted selection across
      // reloads, not just redux-persist's local copy (issue #3712).
      if (defaultChannel) {
        dispatch(setDefaultMessagingChannel(defaultChannel));
      }

      const resolvedDefs =
        defs && Array.isArray(defs) && defs.length > 0 ? defs : FALLBACK_DEFINITIONS;
      setDefinitions(resolvedDefs);
      log('loaded %d channel definitions', resolvedDefs.length);

      if (statusEntries && Array.isArray(statusEntries)) {
        // Read live store state (not the closed-over selector value) so the
        // connecting-guard in `resolveStatusPatch` sees the current status.
        const liveConnections = store.getState().channelConnections.connections;
        for (const entry of statusEntries) {
          // Skip unknown channels from core rather than coercing them into
          // state as if valid (#3794 review).
          if (!isChannelType(entry.channel_id)) {
            log('ignoring unknown channel_id from status sync: %s', entry.channel_id);
            continue;
          }
          const channel = entry.channel_id;
          const authMode = entry.auth_mode as ChannelAuthMode;
          const currentStatus = liveConnections[channel]?.[authMode]?.status;
          const patch = resolveStatusPatch(entry, currentStatus);
          if (!patch) continue;
          dispatch(
            upsertChannelConnection({
              channel,
              authMode,
              patch: {
                status: patch.status,
                lastError: patch.lastError,
                // Only assert capabilities when actually connected; leave them
                // untouched otherwise so a disconnect doesn't wipe them.
                ...(entry.connected ? { capabilities: ['read', 'write'] } : {}),
              },
            })
          );
        }
        log('synced %d status entries', statusEntries.length);
      }
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      if (!cancelled) {
        setDefinitions(FALLBACK_DEFINITIONS);
        setError(`Could not load from backend: ${msg}`);
        log('fallback to local definitions: %s', msg);
      }
    } finally {
      if (!cancelled) setLoading(false);
    }

    return () => {
      cancelled = true;
    };
  }, [dispatch]);

  useEffect(() => {
    void loadDefinitions();
  }, [loadDefinitions]);

  return { definitions, loading, error, refreshDefinitions: loadDefinitions };
}
