/**
 * Skill state event bus — bridges Tauri runtime events to React hooks.
 *
 * When the Rust core emits skill state changes, listeners here trigger
 * re-fetches in the React hooks that consume skill state via RPC.
 */

import { listen } from '@tauri-apps/api/event';

type Listener = (skillId?: string) => void;

const listeners = new Set<Listener>();

/** Subscribe to skill state invalidation events. Returns unsubscribe fn. */
export function onSkillStateChange(fn: Listener): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

/** Notify all listeners that skill state has changed. */
export function emitSkillStateChange(skillId?: string): void {
  for (const fn of listeners) {
    fn(skillId);
  }
}

/** Setup Tauri event listeners that bridge to the skill event bus. */
export async function setupTauriSkillEventBridge(): Promise<() => void> {
  try {
    const unlistenStatus = await listen<{ skill_id?: string }>(
      'runtime:skill-status-changed',
      (event) => {
        emitSkillStateChange(event.payload?.skill_id);
      },
    );

    const unlistenState = await listen<{ skill_id?: string }>(
      'runtime:skill-state-changed',
      (event) => {
        emitSkillStateChange(event.payload?.skill_id);
      },
    );

    return () => {
      unlistenStatus();
      unlistenState();
    };
  } catch {
    // Not in Tauri environment
    return () => {};
  }
}
