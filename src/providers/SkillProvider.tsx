/**
 * Skill Provider — discovers and manages skill lifecycles.
 *
 * On mount (when authenticated): discovers skills from the local skills
 * directory (dev: submodule, prod: ~/.alphahuman/skills/), registers them
 * in Redux, and auto-starts skills with completed setup.
 *
 * In production:
 * - If no local skills exist, downloads from GitHub before discovery.
 * - On every startup, checks for updates (respects a 24-hour cooldown on
 *   the Rust side). If a newer commit is available, re-downloads and
 *   re-discovers.
 */
import { type ReactNode, useEffect, useRef } from 'react';

import { skillManager } from '../lib/skills/manager';
import type { SkillManifest } from '../lib/skills/types';
import { useAppSelector } from '../store/hooks';
import { IS_DEV, SKILLS_GITHUB_REPO, SKILLS_GITHUB_TOKEN } from '../utils/config';

// ---------------------------------------------------------------------------
// Helpers (all lazy-import @tauri-apps/api/core to avoid loading before IPC)
// ---------------------------------------------------------------------------

async function discoverSkills(): Promise<SkillManifest[]> {
  const { invoke } = await import('@tauri-apps/api/core');
  const raw = await invoke<Record<string, unknown>[]>('skill_list_manifests');
  return raw as unknown as SkillManifest[];
}

async function syncSkillsFromGithub(): Promise<void> {
  const { invoke } = await import('@tauri-apps/api/core');
  await invoke('skill_sync_repo', { repo: SKILLS_GITHUB_REPO, githubToken: SKILLS_GITHUB_TOKEN });
}

async function catalogExists(): Promise<boolean> {
  try {
    const { invoke } = await import('@tauri-apps/api/core');
    return await invoke<boolean>('skill_catalog_exists');
  } catch {
    return false;
  }
}

interface UpdateCheckResult {
  needs_update: boolean;
  skipped?: boolean;
  local_sha?: string | null;
  remote_sha?: string;
}

async function checkForUpdates(): Promise<UpdateCheckResult> {
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke<UpdateCheckResult>('skill_check_for_updates', {
    repo: SKILLS_GITHUB_REPO,
    githubToken: SKILLS_GITHUB_TOKEN,
  });
}

// ---------------------------------------------------------------------------
// Provider
// ---------------------------------------------------------------------------

export default function SkillProvider({ children }: { children: ReactNode }) {
  const { token } = useAppSelector(state => state.auth);
  const skillsState = useAppSelector(state => state.skills.skills);
  const initRef = useRef(false);

  useEffect(() => {
    if (!token) return;
    if (initRef.current) return;
    initRef.current = true;

    const registerAndStart = async (manifests: SkillManifest[]) => {
      for (const manifest of manifests) {
        skillManager.registerSkill(manifest);
      }
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
        if (!IS_DEV) {
          // First-time install: no skills at all yet
          const exists = await catalogExists();
          if (!exists) {
            console.log('[SkillProvider] No local skills found, syncing from GitHub...');
            try {
              await syncSkillsFromGithub();
            } catch (syncErr) {
              console.error('[SkillProvider] Failed to sync skills from GitHub:', syncErr);
            }
          }
        }

        // Discover and start whatever is on disk right now
        const manifests = await discoverSkills();
        await registerAndStart(manifests);

        // Background: check for updates (non-blocking)
        if (!IS_DEV) {
          checkForSkillUpdates(registerAndStart);
        }
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

/**
 * Runs in the background after initial discovery. Asks the Rust side
 * whether a newer commit exists on GitHub (respects 24h cooldown).
 * If yes, re-syncs and re-discovers skills.
 */
async function checkForSkillUpdates(onNewSkills: (manifests: SkillManifest[]) => Promise<void>) {
  try {
    const result = await checkForUpdates();

    if (result.skipped) {
      return; // Cooldown still active, nothing to do
    }

    if (!result.needs_update) {
      console.log('[SkillProvider] Skills are up to date');
      return;
    }

    console.log(
      `[SkillProvider] Skills update available: ${result.local_sha?.slice(0, 8) ?? 'none'} → ${result.remote_sha?.slice(0, 8)}`
    );

    // Stop all running skills before re-syncing
    await skillManager.stopAll();

    await syncSkillsFromGithub();

    // Re-discover and re-start
    const manifests = await discoverSkills();
    await onNewSkills(manifests);

    console.log('[SkillProvider] Skills updated and reloaded');
  } catch (err) {
    console.error('[SkillProvider] Background update check failed:', err);
  }
}
