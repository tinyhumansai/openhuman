import type { RootState } from './index';

const PENDING_USER = '__pending__';

function selectCurrentUserId(state: RootState): string {
  return state.user.user?._id ?? PENDING_USER;
}

export const selectSocketStatus = (state: RootState) => {
  const userId = selectCurrentUserId(state);
  const userState = state.socket.byUser[userId];
  return userState?.status ?? 'disconnected';
};

export const selectSocketId = (state: RootState): string | null => {
  const userId = selectCurrentUserId(state);
  const userState = state.socket.byUser[userId];
  return userState?.socketId ?? null;
};
