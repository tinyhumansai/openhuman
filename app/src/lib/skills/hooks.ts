/**
 * React hooks for consuming skill state via RPC.
 *
 * All state is fetched from the Rust core sidecar and cached locally.
 * Tauri events trigger re-fetches for instant reactivity.
 */

import { useCallback, useEffect, useRef, useState } from 'react';

import type { SkillConnectionStatus, SkillHostConnectionState } from './types';
import { onSkillStateChange } from './skillEvents';
import {
  getAllSnapshots,
  getSkillSnapshot,
  listAvailable,
  type AvailableSkillEntryRpc,
  type SkillSnapshotRpc,
} from './skillsApi';

// ---------------------------------------------------------------------------
// Legacy pure function kept for compatibility (used by sync.ts, skillsSyncUi)
// ---------------------------------------------------------------------------

export function deriveConnectionStatus(
  lifecycleStatus: string | undefined,
  setupComplete: boolean | undefined,
  skillState: Record<string, unknown> | undefined,
): SkillConnectionStatus {
  if (!lifecycleStatus || lifecycleStatus === 'installed' || lifecycleStatus === 'stopping') {
    return 'offline';
  }
  if (lifecycleStatus === 'error') return 'error';
  if (lifecycleStatus === 'setup_required' || lifecycleStatus === 'setup_in_progress') {
    return 'setup_required';
  }
  if (lifecycleStatus === 'starting') return 'connecting';

  const hostState = skillState as SkillHostConnectionState | undefined;
  const connStatus = hostState?.connection_status;
  const authStatus = hostState?.auth_status;

  if (!connStatus && !authStatus) {
    if (setupComplete && (lifecycleStatus === 'ready' || lifecycleStatus === 'running')) {
      return 'connected';
    }
    if (!hostState) return 'connecting';
    return 'connecting';
  }

  if (connStatus === 'error' || authStatus === 'error') return 'error';
  if (connStatus === 'connecting' || authStatus === 'authenticating') return 'connecting';
  if (connStatus === 'connected') {
    if (!authStatus || authStatus === 'authenticated') return 'connected';
    if (authStatus === 'not_authenticated') return 'not_authenticated';
  }
  if (connStatus === 'disconnected') {
    return setupComplete ? 'disconnected' : 'setup_required';
  }

  return 'connecting';
}

// ---------------------------------------------------------------------------
// RPC-backed hooks
// ---------------------------------------------------------------------------

/**
 * Fetch a single skill snapshot, re-fetching on skill events and polling
 * periodically (the core sidecar has no push channel to the frontend).
 */
export function useSkillSnapshot(skillId: string | undefined): SkillSnapshotRpc | null {
  const [snap, setSnap] = useState<SkillSnapshotRpc | null>(null);
  const mountedRef = useRef(true);

  const refresh = useCallback(async () => {
    if (!skillId) return;
    try {
      const s = await getSkillSnapshot(skillId);
      if (mountedRef.current) setSnap(s);
    } catch {
      // Skill may not be running yet — that's OK
    }
  }, [skillId]);

  useEffect(() => {
    mountedRef.current = true;
    refresh();
    const unsub = onSkillStateChange((changedId) => {
      if (!changedId || changedId === skillId) refresh();
    });
    // Poll every 3s to catch background state changes from the core sidecar
    const interval = setInterval(refresh, 3000);
    return () => {
      mountedRef.current = false;
      unsub();
      clearInterval(interval);
    };
  }, [skillId, refresh]);

  return snap;
}

/** Fetch all running skill snapshots, re-fetching on skill events and polling. */
export function useAllSkillSnapshots(): SkillSnapshotRpc[] {
  const [snaps, setSnaps] = useState<SkillSnapshotRpc[]>([]);
  const mountedRef = useRef(true);

  const refresh = useCallback(async () => {
    try {
      const s = await getAllSnapshots();
      if (mountedRef.current) setSnaps(s);
    } catch {
      // Core not ready yet
    }
  }, []);

  useEffect(() => {
    mountedRef.current = true;
    refresh();
    const unsub = onSkillStateChange(() => refresh());
    const interval = setInterval(refresh, 3000);
    return () => {
      mountedRef.current = false;
      unsub();
      clearInterval(interval);
    };
  }, [refresh]);

  return snaps;
}

/** Fetch available skills from registry. */
export function useAvailableSkills(): {
  skills: AvailableSkillEntryRpc[];
  loading: boolean;
  refresh: () => Promise<void>;
} {
  const [skills, setSkills] = useState<AvailableSkillEntryRpc[]>([]);
  const [loading, setLoading] = useState(true);
  const mountedRef = useRef(true);

  const refresh = useCallback(async () => {
    try {
      const s = await listAvailable();
      if (mountedRef.current) setSkills(s);
    } catch {
      // Registry not reachable
    } finally {
      if (mountedRef.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    mountedRef.current = true;
    refresh();
    return () => {
      mountedRef.current = false;
    };
  }, [refresh]);

  return { skills, loading, refresh };
}

/**
 * Returns the connection status for a skill.
 * Reads from the Rust-derived `connection_status` field in the snapshot.
 */
export function useSkillConnectionStatus(skillId: string): SkillConnectionStatus {
  const snap = useSkillSnapshot(skillId);
  if (!snap) return 'offline';
  return (snap.connection_status as SkillConnectionStatus) || 'offline';
}

/**
 * Returns the raw skill-pushed state (from reverse RPC state/set).
 */
export function useSkillState<T = Record<string, unknown>>(
  skillId: string,
): T | undefined {
  const snap = useSkillSnapshot(skillId);
  return snap?.state as T | undefined;
}

/**
 * Returns connection status info including error messages.
 */
export function useSkillConnectionInfo(skillId: string): {
  status: SkillConnectionStatus;
  error?: string | null;
  isInitialized: boolean;
} {
  const snap = useSkillSnapshot(skillId);

  if (!snap) {
    return { status: 'offline', error: null, isInitialized: false };
  }

  const status = (snap.connection_status as SkillConnectionStatus) || 'offline';
  const hostState = snap.state as SkillHostConnectionState | undefined;

  let error: string | null | undefined;
  if (status === 'error') {
    error = hostState?.connection_error ?? hostState?.auth_error ?? snap.error ?? null;
  }

  return {
    status,
    error,
    isInitialized: !!hostState?.is_initialized,
  };
}
