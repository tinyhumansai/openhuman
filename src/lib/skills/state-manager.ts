/**
 * Skill State Manager — Zustand Store Per Skill
 *
 * Each skill gets its own Zustand store (vanilla mode, no React dependency).
 * Completely separate from Redux — core app state stays in Redux,
 * skill-specific state lives here.
 */

import { createStore, type StoreApi } from "zustand/vanilla";
import { persist, createJSONStorage } from "zustand/middleware";
import type { SkillStateDefinition } from "./types";
import createDebug from "debug";

const log = createDebug("app:skills:state");

export class SkillStateManager {
  private stores = new Map<string, StoreApi<Record<string, unknown>>>();

  /**
   * Create a Zustand store for a skill.
   * If a store already exists for this skillId, it is destroyed first.
   */
  createStore(skillId: string, definition: SkillStateDefinition): void {
    // Destroy existing store if present
    if (this.stores.has(skillId)) {
      this.destroyStore(skillId);
    }

    const initial = definition.initialState as Record<string, unknown>;

    if (definition.persist) {
      const { name, whitelist, volatileKeys } = definition.persist;

      const store = createStore(
        persist<Record<string, unknown>>(
          () => ({ ...initial }),
          {
            name: `skill-${name}`,
            storage: createJSONStorage(() => localStorage),
            partialize: (state) => {
              if (whitelist && whitelist.length > 0) {
                const partial: Record<string, unknown> = {};
                for (const key of whitelist) {
                  if (key in state) {
                    partial[key] = state[key];
                  }
                }
                return partial;
              }
              return state;
            },
            onRehydrateStorage: () => {
              return (state) => {
                if (state && volatileKeys) {
                  for (const key of volatileKeys) {
                    if (key in initial) {
                      state[key] = initial[key];
                    }
                  }
                }
              };
            },
          },
        ),
      );

      this.stores.set(skillId, store);
    } else {
      const store = createStore<Record<string, unknown>>(() => ({ ...initial }));
      this.stores.set(skillId, store);
    }

    log("Created store for skill %s", skillId);
  }

  /** Get a skill's current state */
  getState<S>(skillId: string): S | undefined {
    const store = this.stores.get(skillId);
    if (!store) return undefined;
    return store.getState() as S;
  }

  /** Update a skill's state (shallow merge) */
  setState<S>(skillId: string, partial: Partial<S>): void {
    const store = this.stores.get(skillId);
    if (!store) return;
    store.setState(partial as Partial<Record<string, unknown>>);
  }

  /** Subscribe to state changes for a skill */
  subscribe(
    skillId: string,
    listener: (state: unknown) => void,
  ): () => void {
    const store = this.stores.get(skillId);
    if (!store) return () => {};
    return store.subscribe(listener);
  }

  /** Destroy a skill's store and clean up */
  destroyStore(skillId: string): void {
    const store = this.stores.get(skillId);
    if (store) {
      store.destroy();
      this.stores.delete(skillId);
      log("Destroyed store for skill %s", skillId);
    }
  }

  /** Check if a store exists for a skill */
  hasStore(skillId: string): boolean {
    return this.stores.has(skillId);
  }

  /** Destroy all stores */
  destroyAll(): void {
    for (const [skillId] of this.stores) {
      this.destroyStore(skillId);
    }
  }
}
