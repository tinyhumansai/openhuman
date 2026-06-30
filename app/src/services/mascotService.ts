// Client for the backend mascot library — GET /mascots (summaries) and
// GET /mascots/:id (full manifest with per-state SVGs + visemes).
//
// Backend: tinyhumansai/backend PR #770. Both endpoints are public
// (manifests only, no compute) so we skip auth.
import type {
  GetMascotResponse,
  ListMascotsResponse,
  MascotDetailUnion,
  MascotSummary,
  RiveMascotDetail,
} from '../features/human/Mascot/backend/types';
import { loadRivBuffer } from '../features/human/Mascot/rivCache';
import { apiClient } from './apiClient';
import { getBackendUrl } from './backendUrl';

export async function fetchMascotList(): Promise<MascotSummary[]> {
  const res = await apiClient.get<ListMascotsResponse>('/mascots', { requireAuth: false });
  return res.data.mascots;
}

export async function fetchMascotDetail(id: string): Promise<MascotDetailUnion> {
  const safe = encodeURIComponent(id.trim());
  if (!safe) throw new Error('mascot id is empty');
  const res = await apiClient.get<GetMascotResponse>(`/mascots/${safe}`, { requireAuth: false });
  return res.data.mascot;
}

/**
 * Resolve a Rive mascot's binary, version-cached in IndexedDB. The backend
 * stamps `version` into both the manifest and the `rivFileUrl` (`?v=`), so the
 * binary is only re-downloaded when that version changes.
 */
export async function loadMascotRivBuffer(detail: RiveMascotDetail): Promise<ArrayBuffer> {
  const base = await getBackendUrl();
  // rivFileUrl is backend-relative (e.g. "/mascots/toshi/riv?v=1.0.0").
  const url = `${base}${detail.rivFileUrl}`;
  return loadRivBuffer(detail.id, detail.version, url);
}

/**
 * Lightweight in-memory cache for manifest fetches. Manifests carry the
 * full SVG bytes for every state (~ tens of KB per mascot) — the WebRTC
 * pipeline keeps them in mongo and the picker UI revisits selections,
 * so a per-id memoization keeps the picker snappy without hammering the
 * backend.
 */
const detailCache = new Map<string, MascotDetailUnion>();

export async function getCachedMascotDetail(id: string): Promise<MascotDetailUnion> {
  const existing = detailCache.get(id);
  if (existing) return existing;
  const detail = await fetchMascotDetail(id);
  detailCache.set(id, detail);
  return detail;
}

export function clearMascotDetailCache(): void {
  detailCache.clear();
}
