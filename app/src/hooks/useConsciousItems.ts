/**
 * useConsciousItems
 *
 * Reads actionable items from the `conscious` memory namespace (populated by
 * the Rust conscious loop) and exposes a trigger to kick off a new analysis run.
 *
 * Data flow:
 *   conscious_loop_run_inner (Rust)
 *     → stores ExtractedActionable JSON docs in `conscious` namespace
 *     → emits `conscious_loop:started` / `conscious_loop:completed`
 *   useConsciousItems (here)
 *     → fetches via memoryQueryNamespace on mount + on completed event
 *     → parses JSON objects out of the formatted context string
 *     → maps to ActionableItem[]
 */
import { listen } from '@tauri-apps/api/event';
import { useCallback, useEffect, useRef, useState } from 'react';

import { getCoreStateSnapshot } from '../lib/coreState/store';
import { getBackendUrl } from '../services/backendUrl';
import type {
  ActionableItem,
  ActionableItemPriority,
  ActionableItemSource,
} from '../types/intelligence';
import { consciousLoopRun, isTauri, memoryQueryNamespace } from '../utils/tauriCommands';

// ─── Types from conscious_loop.rs (mirrored) ────────────────────────────────

interface ExtractedActionable {
  title: string;
  description?: string;
  source: string;
  priority: string;
  actionable: boolean;
  requires_confirmation: boolean;
  has_complex_action: boolean;
  source_label: string;
}

// ─── JSON extraction ─────────────────────────────────────────────────────────

/**
 * Walk the context string and extract all valid JSON objects that look like
 * ExtractedActionable items. Uses brace-depth tracking to handle the full
 * object regardless of whitespace or newlines.
 */
function extractActionablesFromContext(context: string): ExtractedActionable[] {
  const results: ExtractedActionable[] = [];
  const seen = new Set<string>();
  let i = 0;

  while (i < context.length) {
    if (context[i] === '{') {
      let depth = 0;
      let j = i;
      while (j < context.length) {
        if (context[j] === '{') depth++;
        else if (context[j] === '}') {
          depth--;
          if (depth === 0) break;
        }
        j++;
      }
      if (depth === 0) {
        try {
          const candidate = context.slice(i, j + 1);
          const parsed = JSON.parse(candidate) as Record<string, unknown>;
          if (
            typeof parsed.title === 'string' &&
            typeof parsed.source === 'string' &&
            typeof parsed.priority === 'string' &&
            !seen.has(parsed.title)
          ) {
            seen.add(parsed.title);
            results.push(parsed as unknown as ExtractedActionable);
          }
        } catch {
          // not valid JSON — skip
        }
        i = j + 1;
      } else {
        i++;
      }
    } else {
      i++;
    }
  }

  return results;
}

// ─── Mapping ─────────────────────────────────────────────────────────────────

const VALID_SOURCES: ActionableItemSource[] = [
  'email',
  'calendar',
  'telegram',
  'ai_insight',
  'system',
  'trading',
  'security',
];
const VALID_PRIORITIES: ActionableItemPriority[] = ['critical', 'important', 'normal'];

function mapToActionableItem(item: ExtractedActionable, index: number): ActionableItem {
  const source: ActionableItemSource = VALID_SOURCES.includes(item.source as ActionableItemSource)
    ? (item.source as ActionableItemSource)
    : 'ai_insight';

  const priority: ActionableItemPriority = VALID_PRIORITIES.includes(
    item.priority as ActionableItemPriority
  )
    ? (item.priority as ActionableItemPriority)
    : 'normal';

  return {
    id: `conscious-${index}-${item.title.slice(0, 20).replace(/\s+/g, '-')}`,
    title: item.title,
    description: item.description,
    source,
    priority,
    status: 'active',
    createdAt: new Date(),
    updatedAt: new Date(),
    actionable: item.actionable,
    requiresConfirmation: item.requires_confirmation,
    hasComplexAction: item.has_complex_action,
    sourceLabel: item.source_label,
  };
}

// ─── Hook ─────────────────────────────────────────────────────────────────────

export interface UseConsciousItemsResult {
  items: ActionableItem[];
  loading: boolean;
  isRunning: boolean;
  error: string | null;
  refresh: () => Promise<void>;
  triggerAnalysis: () => Promise<void>;
}

export function useConsciousItems(): UseConsciousItemsResult {
  const [items, setItems] = useState<ActionableItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [isRunning, setIsRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Prevent double-fetch on StrictMode double-mount
  const fetchingRef = useRef(false);

  const refresh = useCallback(async () => {
    if (!isTauri() || fetchingRef.current) return;
    fetchingRef.current = true;
    setLoading(true);
    setError(null);
    try {
      const queryResult = await memoryQueryNamespace(
        'conscious',
        'actionable items priority source title description',
        20
      );
      const extracted = extractActionablesFromContext(queryResult.text);
      setItems(extracted.map(mapToActionableItem));
    } catch (err) {
      const msg = err instanceof Error ? err.message : 'Failed to load conscious items';
      setError(msg);
    } finally {
      setLoading(false);
      fetchingRef.current = false;
    }
  }, []);

  const triggerAnalysis = useCallback(async () => {
    const authToken = getCoreStateSnapshot().snapshot.sessionToken;
    if (!isTauri() || !authToken || isRunning) return;
    try {
      await consciousLoopRun(authToken, await getBackendUrl());
    } catch (err) {
      console.warn('[conscious] Failed to trigger analysis:', err);
    }
  }, [isRunning]);

  // Initial fetch
  useEffect(() => {
    refresh();
  }, [refresh]);

  // Listen to conscious loop events
  useEffect(() => {
    if (!isTauri()) return;

    let unlistenStarted: (() => void) | undefined;
    let unlistenCompleted: (() => void) | undefined;

    listen('conscious_loop:started', () => {
      setIsRunning(true);
    })
      .then(fn => {
        unlistenStarted = fn;
      })
      .catch(console.warn);

    listen('conscious_loop:completed', () => {
      setIsRunning(false);
      refresh();
    })
      .then(fn => {
        unlistenCompleted = fn;
      })
      .catch(console.warn);

    listen('conscious_loop:error', () => {
      setIsRunning(false);
    }).catch(console.warn);

    return () => {
      unlistenStarted?.();
      unlistenCompleted?.();
    };
  }, [refresh]);

  return { items, loading, isRunning, error, refresh, triggerAnalysis };
}
