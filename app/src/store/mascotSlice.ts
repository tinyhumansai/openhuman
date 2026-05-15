import { createSlice, type PayloadAction } from '@reduxjs/toolkit';
import { REHYDRATE } from 'redux-persist';

import type { MascotColor } from '../features/human/Mascot/mascotPalette';
import { resetUserScopedState } from './resetActions';

export const SUPPORTED_MASCOT_COLORS: readonly MascotColor[] = [
  'yellow',
  'burgundy',
  'black',
  'navy',
  'green',
];

export const DEFAULT_MASCOT_COLOR: MascotColor = 'yellow';

export interface MascotState {
  color: MascotColor;
}

const initialState: MascotState = { color: DEFAULT_MASCOT_COLOR };

function isMascotColor(value: unknown): value is MascotColor {
  return (
    typeof value === 'string' && (SUPPORTED_MASCOT_COLORS as readonly string[]).includes(value)
  );
}

const mascotSlice = createSlice({
  name: 'mascot',
  initialState,
  reducers: {
    setMascotColor(state, action: PayloadAction<MascotColor>) {
      if (isMascotColor(action.payload)) {
        state.color = action.payload;
      }
    },
  },
  extraReducers: builder => {
    builder.addCase(resetUserScopedState, () => initialState);
    // Guard against unknown/missing color values surviving a rehydrate (e.g.
    // a future build removed a variant that was previously persisted).
    builder.addCase(REHYDRATE, (state, action) => {
      const rehydrateAction = action as {
        type: typeof REHYDRATE;
        key: string;
        payload?: { color?: unknown };
      };
      if (rehydrateAction.key !== 'mascot') return;
      const restored = rehydrateAction.payload?.color;
      state.color = isMascotColor(restored) ? restored : DEFAULT_MASCOT_COLOR;
    });
  },
});

export const { setMascotColor } = mascotSlice.actions;

export const selectMascotColor = (state: { mascot: MascotState }): MascotColor =>
  state.mascot.color;

export { mascotSlice };
export default mascotSlice.reducer;
