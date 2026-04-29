import { createSlice, PayloadAction } from '@reduxjs/toolkit';

import { resetUserScopedState } from './resetActions';

export type SocketConnectionStatus = 'connected' | 'disconnected' | 'connecting';

export interface SocketUserState {
  status: SocketConnectionStatus;
  socketId: string | null;
}

const initialUserState: SocketUserState = { status: 'disconnected', socketId: null };

interface SocketState {
  /** Socket state per user id. Use __pending__ when user not loaded yet. */
  byUser: Record<string, SocketUserState>;
}

const initialState: SocketState = { byUser: {} };

const ensureUserState = (state: SocketState, userId: string): SocketUserState => {
  if (!state.byUser[userId]) {
    state.byUser[userId] = { ...initialUserState };
  }
  return state.byUser[userId];
};

const socketSlice = createSlice({
  name: 'socket',
  initialState,
  reducers: {
    setStatusForUser: (
      state,
      action: PayloadAction<{ userId: string; status: SocketConnectionStatus }>
    ) => {
      const { userId, status } = action.payload;
      const user = ensureUserState(state, userId);
      user.status = status;
      if (status === 'disconnected' || status === 'connecting') {
        user.socketId = null;
      }
    },
    setSocketIdForUser: (
      state,
      action: PayloadAction<{ userId: string; socketId: string | null }>
    ) => {
      const { userId, socketId } = action.payload;
      const user = ensureUserState(state, userId);
      user.socketId = socketId;
    },
    resetForUser: (state, action: PayloadAction<{ userId: string }>) => {
      const { userId } = action.payload;
      state.byUser[userId] = { ...initialUserState };
    },
  },
  extraReducers: builder => {
    builder.addCase(resetUserScopedState, () => initialState);
  },
});

export const { setStatusForUser, setSocketIdForUser, resetForUser } = socketSlice.actions;
export default socketSlice.reducer;
