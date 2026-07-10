/**
 * Frontend client for the long-term goals surface (`openhuman.memory_goals_*`).
 *
 * The Rust handlers persist an editable list of the agent's durable long-term
 * goals to `<workspace>/MEMORY_GOALS.md`. The same list is curated by the
 * background `goals_agent` (enrichment) — user edits here and agent edits stay
 * in lock-step on the same file.
 *
 * Wire shapes: `list` returns the bare `GoalsDoc` ({ items }); the mutation
 * methods wrap their value in `{ result, logs }` when logs are present. The
 * {@link extractItems} helper normalises both shapes so callers always get a
 * `GoalItem[]`.
 */
import debug from 'debug';

import { callCoreRpc } from '../coreRpcClient';

const log = debug('openhuman:goalsApi');

/** A single long-term goal item. */
export interface GoalItem {
  id: string;
  text: string;
}

/** Outcome of an enrichment (reflect) pass. */
export interface ReflectResult {
  ran: boolean;
  summary: string;
  items: GoalItem[];
}

/** Drop `undefined` params so the wire payload stays clean. */
function pruneParams<T extends Record<string, unknown>>(params: T): Partial<T> {
  const out: Partial<T> = {};
  for (const [k, v] of Object.entries(params)) {
    if (v !== undefined) (out as Record<string, unknown>)[k] = v;
  }
  return out;
}

/** Pull the goal items out of any of the handler response shapes. */
function extractItems(res: unknown): GoalItem[] {
  if (!res || typeof res !== 'object') return [];
  // Unwrap the RpcOutcome `{ result, logs }` envelope when present.
  const value =
    'result' in (res as Record<string, unknown>) ? (res as { result: unknown }).result : res;
  if (!value || typeof value !== 'object') return [];
  const v = value as Record<string, unknown>;
  // Direct GoalsDoc ({ items }) or AddResult/ReflectResult ({ goals: { items } }).
  const items = Array.isArray(v.items)
    ? v.items
    : Array.isArray((v.goals as { items?: unknown })?.items)
      ? (v.goals as { items: unknown[] }).items
      : [];
  return items.filter(
    (i): i is GoalItem =>
      !!i && typeof (i as GoalItem).id === 'string' && typeof (i as GoalItem).text === 'string'
  );
}

export const goalsApi = {
  list: async (): Promise<GoalItem[]> => {
    log('list');
    const res = await callCoreRpc<unknown>({ method: 'openhuman.memory_goals_list', params: {} });
    return extractItems(res);
  },

  add: async (text: string): Promise<GoalItem[]> => {
    log('add');
    const res = await callCoreRpc<unknown>({
      method: 'openhuman.memory_goals_add',
      params: { text },
    });
    return extractItems(res);
  },

  edit: async (id: string, text: string): Promise<GoalItem[]> => {
    log('edit id=%s', id);
    const res = await callCoreRpc<unknown>({
      method: 'openhuman.memory_goals_edit',
      params: { id, text },
    });
    return extractItems(res);
  },

  remove: async (id: string): Promise<GoalItem[]> => {
    log('delete id=%s', id);
    const res = await callCoreRpc<unknown>({
      method: 'openhuman.memory_goals_delete',
      params: { id },
    });
    return extractItems(res);
  },

  reflect: async (context?: string): Promise<ReflectResult> => {
    log('reflect hasContext=%s', Boolean(context));
    const res = await callCoreRpc<unknown>({
      method: 'openhuman.memory_goals_reflect',
      params: pruneParams({ context }),
      // Enrichment runs a full agent turn — give it room beyond the default.
      timeoutMs: 180_000,
    });
    const envelope =
      res && typeof res === 'object' && 'result' in (res as Record<string, unknown>)
        ? (res as { result: Record<string, unknown> }).result
        : ((res as Record<string, unknown>) ?? {});
    return {
      ran: Boolean(envelope?.ran),
      summary: typeof envelope?.summary === 'string' ? envelope.summary : '',
      items: extractItems(res),
    };
  },
};
