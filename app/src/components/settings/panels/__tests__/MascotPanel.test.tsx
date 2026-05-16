import { configureStore } from '@reduxjs/toolkit';
import { fireEvent, render, screen } from '@testing-library/react';
import { Provider } from 'react-redux';
import { MemoryRouter } from 'react-router-dom';
import { REHYDRATE } from 'redux-persist';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import mascotReducer, { DEFAULT_MASCOT_COLOR, setMascotColor } from '../../../../store/mascotSlice';
import MascotPanel from '../MascotPanel';

const { mockNavigateBack } = vi.hoisted(() => ({ mockNavigateBack: vi.fn() }));

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateBack: mockNavigateBack,
    breadcrumbs: [{ label: 'Settings' }],
  }),
}));

function buildStore() {
  return configureStore({ reducer: { mascot: mascotReducer } });
}

function renderPanel(store = buildStore()) {
  return {
    store,
    ...render(
      <Provider store={store}>
        <MemoryRouter>
          <MascotPanel />
        </MemoryRouter>
      </Provider>
    ),
  };
}

describe('MascotPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders a radio swatch for each supported color', () => {
    renderPanel();
    expect(screen.getByRole('radiogroup', { name: 'Mascot color' })).toBeInTheDocument();
    for (const label of ['Yellow', 'Burgundy', 'Black', 'Navy', 'Green']) {
      expect(screen.getByRole('radio', { name: label })).toBeInTheDocument();
    }
  });

  it('marks the currently selected color as aria-checked', () => {
    const store = buildStore();
    store.dispatch(setMascotColor('navy'));
    renderPanel(store);
    expect(screen.getByRole('radio', { name: 'Navy' })).toHaveAttribute('aria-checked', 'true');
    expect(screen.getByRole('radio', { name: 'Yellow' })).toHaveAttribute('aria-checked', 'false');
  });

  it('dispatches setMascotColor when a swatch is clicked', () => {
    const { store } = renderPanel();
    fireEvent.click(screen.getByRole('radio', { name: 'Burgundy' }));
    expect(store.getState().mascot.color).toBe('burgundy');
  });

  it('is a no-op when clicking the already-selected color', () => {
    const store = buildStore();
    store.dispatch(setMascotColor('green'));
    const dispatchSpy = vi.spyOn(store, 'dispatch');
    renderPanel(store);
    fireEvent.click(screen.getByRole('radio', { name: 'Green' }));
    // No additional dispatches beyond what React-Redux did to subscribe.
    expect(dispatchSpy).not.toHaveBeenCalled();
    expect(store.getState().mascot.color).toBe('green');
  });

  it('invokes navigateBack from the header back button', () => {
    renderPanel();
    fireEvent.click(screen.getByLabelText('Back'));
    expect(mockNavigateBack).toHaveBeenCalledTimes(1);
  });
});

// Batch-5: rehydrate cases + unknown-color fallback (issue#1651, pr#1667)
describe('MascotPanel — mascotSlice rehydrate guard', () => {
  it('restores a known persisted color from a REHYDRATE action', () => {
    const store = configureStore({ reducer: { mascot: mascotReducer } });
    store.dispatch({ type: REHYDRATE, key: 'mascot', payload: { color: 'burgundy' } });
    expect(store.getState().mascot.color).toBe('burgundy');
  });

  it('falls back to yellow when REHYDRATE contains an unknown color string', () => {
    const store = configureStore({ reducer: { mascot: mascotReducer } });
    store.dispatch({ type: REHYDRATE, key: 'mascot', payload: { color: 'hot-pink' } });
    expect(store.getState().mascot.color).toBe(DEFAULT_MASCOT_COLOR);
  });

  it('falls back to yellow when REHYDRATE payload is missing the color field', () => {
    const store = configureStore({ reducer: { mascot: mascotReducer } });
    store.dispatch({ type: REHYDRATE, key: 'mascot', payload: {} });
    expect(store.getState().mascot.color).toBe(DEFAULT_MASCOT_COLOR);
  });

  it('falls back to yellow when REHYDRATE payload is null', () => {
    const store = configureStore({ reducer: { mascot: mascotReducer } });
    store.dispatch({ type: REHYDRATE, key: 'mascot', payload: null });
    expect(store.getState().mascot.color).toBe(DEFAULT_MASCOT_COLOR);
  });

  it('ignores REHYDRATE actions for other slice keys', () => {
    const store = configureStore({ reducer: { mascot: mascotReducer } });
    store.dispatch(setMascotColor('navy'));
    store.dispatch({ type: REHYDRATE, key: 'someOtherSlice', payload: { color: 'green' } });
    // Should remain navy — we only handle key === 'mascot'.
    expect(store.getState().mascot.color).toBe('navy');
  });

  it('renders the rehydrated color as selected in the panel', () => {
    const store = configureStore({ reducer: { mascot: mascotReducer } });
    store.dispatch({ type: REHYDRATE, key: 'mascot', payload: { color: 'green' } });
    render(
      <Provider store={store}>
        <MemoryRouter>
          <MascotPanel />
        </MemoryRouter>
      </Provider>
    );
    expect(screen.getByRole('radio', { name: 'Green' })).toHaveAttribute('aria-checked', 'true');
    expect(screen.getByRole('radio', { name: 'Yellow' })).toHaveAttribute('aria-checked', 'false');
  });
});
