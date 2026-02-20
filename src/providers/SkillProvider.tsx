/**
 * Skill Provider — discovers and manages skill lifecycles.
 *
 * On mount (when authenticated): discovers skills from the QuickJS runtime
 * engine, registers them in Redux, and auto-starts skills with completed setup.
 */
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { type ReactNode, useEffect, useRef } from 'react';

import { skillManager } from '../lib/skills/manager';
import type { SkillManifest } from '../lib/skills/types';
import { buildManualSentryEvent, enqueueError } from '../services/errorReportQueue';
import { useAppDispatch, useAppSelector } from '../store/hooks';
import {
  setGmailEmails,
  setGmailProfile,
  type GmailEmailSummary,
  type GmailProfile,
} from '../store/gmailSlice';
import { setSkillError, setSkillState, setSkillStatus } from '../store/skillsSlice';
import {
  syncGmailMetadataToBackend,
  type GmailStateForSync,
} from '../lib/skills/gmailMetadataSync';
import { DEV_AUTO_LOAD_SKILL, IS_DEV } from '../utils/config';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async function discoverSkills(): Promise<SkillManifest[]> {
  const raw = await invoke<Array<Record<string, unknown>>>('runtime_discover_skills');

  const manifests: SkillManifest[] = raw.map(m => ({
    id: m.id as string,
    name: m.name as string,
    version: (m.version as string) || '0.0.0',
    description: (m.description as string) || '',
    runtime: 'quickjs' as const,
    entry: m.entry as string | undefined,
    ignoreInProduction: (m.ignoreInProduction as boolean) ?? false,
    setup: m.setup
      ? {
          required: ((m.setup as Record<string, unknown>).required as boolean) ?? false,
          label: (m.setup as Record<string, unknown>).label as string | undefined,
          oauth: (m.setup as Record<string, unknown>).oauth as
            | { provider: string; scopes: string[]; apiBaseUrl: string }
            | undefined,
        }
      : undefined,
  }));

  // In production, filter out skills marked as dev-only
  if (!IS_DEV) {
    return manifests.filter(m => !m.ignoreInProduction);
  }

  return manifests;
}

// ---------------------------------------------------------------------------
// Provider
// ---------------------------------------------------------------------------

/** Normalize event payload: Rust emits { skillId, state }; ensure we get both. */
function parseSkillStatePayload(
  payload: unknown
): { skillId: string; state: Record<string, unknown> } | null {
  if (payload == null || typeof payload !== 'object') return null;
  const raw = payload as Record<string, unknown>;
  const skillId = raw.skillId as string | undefined;
  const state = (raw.state ?? raw) as Record<string, unknown> | undefined;
  if (!skillId || state == null || typeof state !== 'object') return null;
  return { skillId, state };
}

/** Sync profile and emails from gmail skill state into gmailSlice and send to backend via socket. */
function syncGmailStateToSlice(
  gmailState: Record<string, unknown> | undefined,
  dispatch: ReturnType<typeof useAppDispatch>
): void {
  if (!gmailState || typeof gmailState !== 'object') return;
  dispatch(
    setGmailProfile(
      gmailState.profile !== undefined && gmailState.profile != null
        ? (gmailState.profile as GmailProfile)
        : null
    )
  );
  dispatch(
    setGmailEmails(
      Array.isArray(gmailState.emails) ? (gmailState.emails as GmailEmailSummary[]) : []
    )
  );
  syncGmailMetadataToBackend(gmailState as GmailStateForSync);
}

export default function SkillProvider({ children }: { children: ReactNode }) {
  const { token } = useAppSelector(state => state.auth);
  const skillsState = useAppSelector(state => state.skills.skills);
  const skillStates = useAppSelector(state => state.skills.skillStates);
  const dispatch = useAppDispatch();
  const initRef = useRef(false);

  // Keep gmailSlice in sync with skills.skillStates.gmail (event handler + rehydration)
  const gmailSkillState = skillStates?.gmail as Record<string, unknown> | undefined;
  useEffect(() => {
    if (!gmailSkillState) return;
    syncGmailStateToSlice(gmailSkillState, dispatch);
  }, [gmailSkillState, dispatch]);

  // Listen for skill state changes emitted from the Rust runtime event loop
  useEffect(() => {
    let unlisten: (() => void) | undefined;

    listen<{ skillId: string; state: Record<string, unknown> }>('skill-state-changed', event => {
      const parsed = parseSkillStatePayload(event.payload);
      if (!parsed) return;
      const { skillId, state: newState } = parsed;
      dispatch(setSkillState({ skillId, state: newState }));

      // Transfer Gmail skill state to gmail store (also synced by effect from skillStates.gmail)
      if (skillId === 'gmail') {
        syncGmailStateToSlice(newState, dispatch);
      }
    })
      .then(fn => {
        unlisten = fn;
      })
      .catch(err => {
        console.error('[SkillProvider] Failed to listen for skill-state-changed:', err);
      });

    return () => {
      unlisten?.();
    };
  }, [dispatch]);

  // Fallback: when gmail skill is ready, fetch state from backend (covers events missed before listener attached)
  const gmailStatus = skillsState?.gmail?.status;
  useEffect(() => {
    if (gmailStatus !== 'ready') return;
    const timeoutId = window.setTimeout(() => {
      invoke<{ state?: Record<string, unknown> } | null>('runtime_get_skill_state', {
        skillId: 'gmail',
      })
        .then(snapshot => {
          if (snapshot?.state && typeof snapshot.state === 'object') {
            dispatch(setSkillState({ skillId: 'gmail', state: snapshot.state }));
            syncGmailStateToSlice(snapshot.state, dispatch);
          }
        })
        .catch(() => {});
    }, 800);
    return () => window.clearTimeout(timeoutId);
  }, [gmailStatus, dispatch]);

  // Listen for skill runtime errors and surface them in the error notification
  useEffect(() => {
    let unlisten: (() => void) | undefined;

    listen<{ skill_id: string; status: string; error?: string; name?: string }>(
      'runtime:skill-status-changed',
      event => {
        const { skill_id, status, error, name } = event.payload;
        if (status === 'error' && error) {
          dispatch(setSkillError({ skillId: skill_id, error }));
          enqueueError({
            id: crypto.randomUUID(),
            timestamp: Date.now(),
            source: 'skill',
            title: `Skill Error: ${name ?? skill_id}`,
            message: error,
            sentryEvent: buildManualSentryEvent(
              { type: 'SkillRuntimeError', value: error },
              { skill_id, ...(name ? { skill_name: name } : {}) }
            ),
          });
        } else if (status === 'stopped' || status === 'pending') {
          // Skill process has stopped — reset to "installed" so the UI
          // shows the Enable button instead of staying in setup mode.
          dispatch(setSkillStatus({ skillId: skill_id, status: 'installed' }));
        }
      }
    )
      .then(fn => {
        unlisten = fn;
      })
      .catch(err => {
        console.error('[SkillProvider] Failed to listen for runtime:skill-status-changed:', err);
      });

    return () => {
      unlisten?.();
    };
  }, [dispatch]);

  useEffect(() => {
    if (!token) return;
    if (initRef.current) return;
    initRef.current = true;

    const registerAndStart = async (manifests: SkillManifest[]) => {
      // Register all discovered skills
      for (const manifest of manifests) {
        skillManager.registerSkill(manifest);
      }

      // Auto-start skill specified in DEV_AUTO_LOAD_SKILL env variable (dev only)
      if (DEV_AUTO_LOAD_SKILL) {
        const autoLoadManifest = manifests.find(m => m.id === DEV_AUTO_LOAD_SKILL);
        if (autoLoadManifest) {
          console.log(`[SkillProvider] Auto-loading skill from env: ${DEV_AUTO_LOAD_SKILL}`);
          try {
            await skillManager.startSkill(autoLoadManifest);
            console.log(`[SkillProvider] Successfully auto-loaded skill: ${DEV_AUTO_LOAD_SKILL}`);
          } catch (err) {
            console.error(`[SkillProvider] Failed to auto-load skill ${DEV_AUTO_LOAD_SKILL}:`, err);
          }
        } else {
          console.warn(
            `[SkillProvider] DEV_AUTO_LOAD_SKILL="${DEV_AUTO_LOAD_SKILL}" specified but skill not found in discovered skills`
          );
        }
      }

      // Auto-start skills with completed setup
      for (const manifest of manifests) {
        const existing = skillsState[manifest.id];
        if (existing?.setupComplete) {
          skillManager.startSkill(manifest).catch(err => {
            console.error(`[SkillProvider] Failed to start ${manifest.id}:`, err);
          });
        }
      }
    };

    const init = async () => {
      try {
        // Discover skills from the QuickJS runtime engine
        const manifests = await discoverSkills();
        await registerAndStart(manifests);
      } catch (err) {
        console.error('[SkillProvider] Failed to discover skills:', err);
      }
    };

    init();

    return () => {
      skillManager.stopAll().catch(console.error);
      initRef.current = false;
    };
  }, [token]); // eslint-disable-line react-hooks/exhaustive-deps

  return <>{children}</>;
}
