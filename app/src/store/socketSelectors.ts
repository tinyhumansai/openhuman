import { getCoreStateSnapshot } from '../lib/coreState/store';
import type { RootState } from './index';

const PENDING_USER = '__pending__';

/**
 * Derive the socket user ID from the JWT token — must match the key used
 * by socketService.ts when writing to byUser[].
 */
function selectSocketUserId(_state: RootState): string {
  const token = getCoreStateSnapshot().snapshot.sessionToken;
  if (!token) return PENDING_USER;

  try {
    const parts = token.split('.');
    if (parts.length !== 3) return PENDING_USER;
    const payloadBase64 = parts[1].replace(/-/g, '+').replace(/_/g, '/');
    const payloadJson = atob(payloadBase64);
    const payload = JSON.parse(payloadJson);
    return payload.tgUserId || payload.userId || payload.sub || PENDING_USER;
  } catch {
    return PENDING_USER;
  }
}

export const selectSocketStatus = (state: RootState) => {
  const userId = selectSocketUserId(state);
  const userState = state.socket.byUser[userId];
  return userState?.status ?? 'disconnected';
};

export const selectSocketId = (state: RootState): string | null => {
  const userId = selectSocketUserId(state);
  const userState = state.socket.byUser[userId];
  return userState?.socketId ?? null;
};
