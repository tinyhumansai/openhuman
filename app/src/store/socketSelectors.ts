import { getCoreStateSnapshot } from '../lib/coreState/store';
import type { RootState } from './index';

const PENDING_USER = '__pending__';

/**
 * Derive the socket user ID — must match the key used by
 * socketService.ts when writing to byUser[].
 */
function selectSocketUserId(_state: RootState): string {
  return getCoreStateSnapshot().snapshot?.auth?.userId ?? PENDING_USER;
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
