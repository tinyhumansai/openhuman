// Client for the backend mascot library — GET /mascots (summaries) and
// GET /mascots/:id (full manifest with per-state SVGs + visemes).
//
// Backend: tinyhumansai/backend PR #770. Both endpoints are public
// (manifests only, no compute) so we skip auth.
import type {
  GetMascotResponse,
  ListMascotsResponse,
  MascotDetail,
  MascotSummary,
} from '../features/human/Mascot/backend/types';
import { apiClient } from './apiClient';

export async function fetchMascotList(): Promise<MascotSummary[]> {
  const res = await apiClient.get<ListMascotsResponse>('/mascots', { requireAuth: false });
  return res.data.mascots;
}

export async function fetchMascotDetail(id: string): Promise<MascotDetail> {
  const safe = encodeURIComponent(id.trim());
  if (!safe) throw new Error('mascot id is empty');
  const res = await apiClient.get<GetMascotResponse>(`/mascots/${safe}`, { requireAuth: false });
  return res.data.mascot;
}

/**
 * Lightweight in-memory cache for manifest fetches. Manifests carry the
 * full SVG bytes for every state (~ tens of KB per mascot) — the WebRTC
 * pipeline keeps them in mongo and the picker UI revisits selections,
 * so a per-id memoization keeps the picker snappy without hammering the
 * backend.
 */
const detailCache = new Map<string, MascotDetail>();

export async function getCachedMascotDetail(id: string): Promise<MascotDetail> {
  const existing = detailCache.get(id);
  if (existing) return existing;
  const detail = await fetchMascotDetail(id);
  detailCache.set(id, detail);
  return detail;
}

export function clearMascotDetailCache(): void {
  detailCache.clear();
}
