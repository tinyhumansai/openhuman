/**
 * Skill Provider — discovers and manages skill lifecycles.
 *
 * On mount (when authenticated): discovers skills from the V8 runtime
 * engine, registers them in Redux, and auto-starts skills with completed setup.
 *
 * The Rust V8 engine handles skill discovery and auto-start independently.
 * This provider bridges the Rust engine state with the frontend Redux store.
 */
import { type ReactNode, useEffect, useRef } from 'react';

import { skillManager } from '../lib/skills/manager';
import type { SkillManifest } from '../lib/skills/types';
import { useAppSelector } from '../store/hooks';
import { DEV_AUTO_LOAD_SKILL } from '../utils/config';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async function discoverSkills(): Promise<SkillManifest[]> {
  const { invoke } = await import('@tauri-apps/api/core');
  const raw = await invoke<Array<Record<string, unknown>>>('runtime_discover_skills');
  // Map the V8 manifest format to SkillManifest
  return raw.map(m => ({
    id: m.id as string,
    name: m.name as string,
    version: (m.version as string) || '0.0.0',
    description: (m.description as string) || '',
    runtime: 'v8' as const,
    entry: m.entry as string | undefined,
    setup: m.setup
      ? {
          required: ((m.setup as Record<string, unknown>).required as boolean) ?? false,
          label: (m.setup as Record<string, unknown>).label as string | undefined,
        }
      : undefined,
  }));
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
        // Discover skills from the V8 runtime engine
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
