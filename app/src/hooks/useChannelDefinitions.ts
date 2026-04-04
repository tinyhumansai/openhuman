import debug from 'debug';
import { useCallback, useEffect, useState } from 'react';

import { FALLBACK_DEFINITIONS } from '../lib/channels/definitions';
import { channelConnectionsApi } from '../services/api/channelConnectionsApi';
import {
  completeBreakingMigration,
  upsertChannelConnection,
} from '../store/channelConnectionsSlice';
import { useAppDispatch, useAppSelector } from '../store/hooks';
import type { ChannelAuthMode, ChannelDefinition, ChannelType } from '../types/channels';

const log = debug('channels:definitions');

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
      const [defs, statusEntries] = await Promise.all([
        channelConnectionsApi.listDefinitions().catch(() => null),
        channelConnectionsApi.listStatus().catch(() => null),
      ]);
      if (cancelled) return;

      const resolvedDefs =
        defs && Array.isArray(defs) && defs.length > 0 ? defs : FALLBACK_DEFINITIONS;
      setDefinitions(resolvedDefs);
      log('loaded %d channel definitions', resolvedDefs.length);

      if (statusEntries && Array.isArray(statusEntries)) {
        for (const entry of statusEntries) {
          const channel = entry.channel_id as ChannelType;
          const authMode = entry.auth_mode as ChannelAuthMode;
          if (entry.connected) {
            dispatch(
              upsertChannelConnection({
                channel,
                authMode,
                patch: { status: 'connected', capabilities: ['read', 'write'] },
              })
            );
          }
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
