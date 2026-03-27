/**
 * Test utilities — provides a renderWithProviders helper that wraps
 * components in a fresh Redux store + MemoryRouter for isolated testing.
 */
import { combineReducers, configureStore } from '@reduxjs/toolkit';
import { render, type RenderOptions } from '@testing-library/react';
import type { PropsWithChildren, ReactElement } from 'react';
import { Provider } from 'react-redux';
import { MemoryRouter } from 'react-router-dom';

import authReducer from '../store/authSlice';
import socketReducer from '../store/socketSlice';
import teamReducer from '../store/teamSlice';
import userReducer from '../store/userSlice';

/**
 * Creates a fresh Redux store for testing.
 * Uses raw (non-persisted) reducers to avoid persist complexity in tests.
 */
const testRootReducer = combineReducers({
  auth: authReducer,
  socket: socketReducer,
  user: userReducer,
  team: teamReducer,
});

export function createTestStore(preloadedState?: Record<string, unknown>) {
  return configureStore({
    reducer: testRootReducer,
    preloadedState: preloadedState as never,
  });
}

type TestStore = ReturnType<typeof createTestStore>;

interface ExtendedRenderOptions extends Omit<RenderOptions, 'queries'> {
  preloadedState?: Record<string, unknown>;
  store?: TestStore;
  initialEntries?: string[];
}

/**
 * Render a component wrapped in Redux Provider + MemoryRouter.
 */
export function renderWithProviders(
  ui: ReactElement,
  {
    preloadedState,
    store = createTestStore(preloadedState),
    initialEntries = ['/'],
    ...renderOptions
  }: ExtendedRenderOptions = {}
) {
  function Wrapper({ children }: PropsWithChildren) {
    return (
      <Provider store={store}>
        <MemoryRouter initialEntries={initialEntries}>{children}</MemoryRouter>
      </Provider>
    );
  }

  return { store, ...render(ui, { wrapper: Wrapper, ...renderOptions }) };
}
