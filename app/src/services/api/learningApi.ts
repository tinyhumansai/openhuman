/**
 * Frontend client for the ambient personalization cache (`openhuman.learning_*`).
 *
 * Facets are scored preferences / identity / tooling / veto / goal / channel
 * rows. Pin shields a fact from decay; forget drops it and blocks re-promotion.
 * The master `learning.enabled` switch gates capture → score → prompt inject.
 */
import debug from 'debug';

import { callCoreRpc } from '../coreRpcClient';

const log = debug('openhuman:learningApi');

/** A single learned facet from `learning_list_facets`. */
export interface LearningFacet {
  key: string;
  value: string;
  state: string;
  user_state: string;
  stability: number;
  confidence?: number;
  evidence_count?: number;
  class?: string | null;
  first_seen_at?: number;
  last_seen_at?: number;
}

export interface LearningSettings {
  enabled: boolean;
}

export interface CacheStats {
  total: number;
  active?: number;
  provisional?: number;
  candidate?: number;
  dropped?: number;
  by_class?: Record<string, number>;
}

/** Split a full facet key (`style/verbosity`) into RPC class + key suffix. */
export function splitFacetKey(fullKey: string): { class: string; key: string } {
  const i = fullKey.indexOf('/');
  if (i <= 0) return { class: 'other', key: fullKey };
  return { class: fullKey.slice(0, i), key: fullKey.slice(i + 1) };
}

function unwrapResult(res: unknown): unknown {
  if (!res || typeof res !== 'object') return res;
  if ('result' in (res as Record<string, unknown>)) {
    return (res as { result: unknown }).result;
  }
  return res;
}

function asFacet(raw: unknown): LearningFacet | null {
  if (!raw || typeof raw !== 'object') return null;
  const f = raw as Record<string, unknown>;
  if (typeof f.key !== 'string' || typeof f.value !== 'string') return null;
  return {
    key: f.key,
    value: f.value,
    state: typeof f.state === 'string' ? f.state : 'active',
    user_state: typeof f.user_state === 'string' ? f.user_state : 'auto',
    stability: typeof f.stability === 'number' ? f.stability : 0,
    confidence: typeof f.confidence === 'number' ? f.confidence : undefined,
    evidence_count: typeof f.evidence_count === 'number' ? f.evidence_count : undefined,
    class: typeof f.class === 'string' ? f.class : null,
    first_seen_at: typeof f.first_seen_at === 'number' ? f.first_seen_at : undefined,
    last_seen_at: typeof f.last_seen_at === 'number' ? f.last_seen_at : undefined,
  };
}

export const learningApi = {
  listFacets: async (classFilter?: string): Promise<LearningFacet[]> => {
    log('listFacets class=%s', classFilter ?? '(all)');
    const res = await callCoreRpc<unknown>({
      method: 'openhuman.learning_list_facets',
      params: classFilter ? { class: classFilter } : {},
    });
    const body = unwrapResult(res) as { facets?: unknown[] } | null;
    const raw = Array.isArray(body?.facets) ? body.facets : [];
    return raw.map(asFacet).filter((f): f is LearningFacet => f !== null);
  },

  pinFacet: async (fullKey: string): Promise<void> => {
    const { class: cls, key } = splitFacetKey(fullKey);
    log('pinFacet %s/%s', cls, key);
    await callCoreRpc({ method: 'openhuman.learning_pin_facet', params: { class: cls, key } });
  },

  unpinFacet: async (fullKey: string): Promise<void> => {
    const { class: cls, key } = splitFacetKey(fullKey);
    log('unpinFacet %s/%s', cls, key);
    await callCoreRpc({ method: 'openhuman.learning_unpin_facet', params: { class: cls, key } });
  },

  forgetFacet: async (fullKey: string): Promise<void> => {
    const { class: cls, key } = splitFacetKey(fullKey);
    log('forgetFacet %s/%s', cls, key);
    await callCoreRpc({ method: 'openhuman.learning_forget_facet', params: { class: cls, key } });
  },

  rebuildCache: async (): Promise<void> => {
    log('rebuildCache');
    await callCoreRpc({ method: 'openhuman.learning_rebuild_cache', params: {} });
  },

  cacheStats: async (): Promise<CacheStats> => {
    log('cacheStats');
    const res = await callCoreRpc<unknown>({
      method: 'openhuman.learning_cache_stats',
      params: {},
    });
    const body = unwrapResult(res) as Record<string, unknown> | null;
    return {
      total: typeof body?.total === 'number' ? body.total : 0,
      active: typeof body?.active === 'number' ? body.active : undefined,
      provisional: typeof body?.provisional === 'number' ? body.provisional : undefined,
      candidate: typeof body?.candidate === 'number' ? body.candidate : undefined,
      dropped: typeof body?.dropped === 'number' ? body.dropped : undefined,
      by_class:
        body?.by_class && typeof body.by_class === 'object'
          ? (body.by_class as Record<string, number>)
          : undefined,
    };
  },

  getSettings: async (): Promise<LearningSettings> => {
    log('getSettings');
    const res = await callCoreRpc<unknown>({
      method: 'openhuman.learning_get_settings',
      params: {},
    });
    const body = unwrapResult(res) as { enabled?: boolean } | null;
    return { enabled: Boolean(body?.enabled) };
  },

  updateSettings: async (enabled: boolean): Promise<LearningSettings> => {
    log('updateSettings enabled=%s', enabled);
    const res = await callCoreRpc<unknown>({
      method: 'openhuman.learning_update_settings',
      params: { enabled },
    });
    const body = unwrapResult(res) as { enabled?: boolean } | null;
    return { enabled: Boolean(body?.enabled) };
  },
};
