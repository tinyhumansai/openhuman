/**
 * Test utilities — provides a renderWithProviders helper that wraps
 * components in a fresh Redux store + MemoryRouter for isolated testing.
 */
import { combineReducers, configureStore } from '@reduxjs/toolkit';
import { render, type RenderOptions } from '@testing-library/react';
import type { PropsWithChildren, ReactElement } from 'react';
import { createPortal } from 'react-dom';
import { Provider } from 'react-redux';
import { MemoryRouter } from 'react-router-dom';

import { SidebarSlotOutlet, SidebarSlotProvider } from '../components/layout/shell/SidebarSlot';
import { getCoreStateSnapshot } from '../lib/coreState/store';
import { CoreStateContext } from '../providers/coreStateContext';
import accountsReducer from '../store/accountsSlice';
import backendMeetReducer from '../store/backendMeetSlice';
import channelConnectionsReducer from '../store/channelConnectionsSlice';
import companionReducer from '../store/companionSlice';
import connectivityReducer from '../store/connectivitySlice';
import coreModeReducer from '../store/coreModeSlice';
import layoutReducer from '../store/layoutSlice';
import localeReducer from '../store/localeSlice';
import mascotReducer from '../store/mascotSlice';
import notificationReducer from '../store/notificationSlice';
import personaReducer from '../store/personaSlice';
import { pttReducer } from '../store/pttSlice';
import socketReducer from '../store/socketSlice';
import themeReducer from '../store/themeSlice';

/**
 * Creates a fresh Redux store for testing.
 * Uses raw (non-persisted) reducers to avoid persist complexity in tests.
 *
 * `mascot` is wired in for the mascot voice picker (issue #1762): the
 * VoicePanel reads + dispatches against this slice, and useSelector
 * would throw on a missing reducer without a stub here. `persona` is wired
 * in for the same reason (issue #2345): PersonaPanel reads + dispatches
 * against it. `backendMeet` is wired in for MeetingBotsCard which reads
 * meeting status from this slice.
 */
const testRootReducer = combineReducers({
  accounts: accountsReducer,
  backendMeet: backendMeetReducer,
  channelConnections: channelConnectionsReducer,
  companion: companionReducer,
  connectivity: connectivityReducer,
  coreMode: coreModeReducer,
  layout: layoutReducer,
  locale: localeReducer,
  mascot: mascotReducer,
  notifications: notificationReducer,
  persona: personaReducer,
  ptt: pttReducer,
  socket: socketReducer,
  theme: themeReducer,
});

export function createTestStore(preloadedState?: Record<string, unknown>) {
  return configureStore({ reducer: testRootReducer, preloadedState: preloadedState as never });
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
  const coreStateStub = {
    ...getCoreStateSnapshot(),
    refresh: async () => {},
    refreshTeams: async () => {},
    refreshTeamMembers: async () => {},
    refreshTeamInvites: async () => {},
    setAnalyticsEnabled: async () => {},
    setMeetAutoOrchestratorHandoff: async () => {},
    setOnboardingCompletedFlag: async () => {},
    setEncryptionKey: async () => {},
    patchSnapshot: () => {},
    setOnboardingTasks: async () => {},
    storeSessionToken: async () => {},
    clearSession: async () => {},
  };

  function Wrapper({ children }: PropsWithChildren) {
    return (
      <Provider store={store}>
        <CoreStateContext.Provider value={coreStateStub}>
          <MemoryRouter initialEntries={initialEntries}>
            {/* Provide the root sidebar slot so pages that project their nav via
                SidebarContent (Settings/Brain/Connections/Chat) render it. The
                outlet is portaled to document.body so it doesn't become the
                render container's firstChild (which would break tests asserting
                a null/empty render); `screen` queries still find projected content. */}
            <SidebarSlotProvider>
              {createPortal(<SidebarSlotOutlet />, document.body)}
              {children}
            </SidebarSlotProvider>
          </MemoryRouter>
        </CoreStateContext.Provider>
      </Provider>
    );
  }

  return { store, ...render(ui, { wrapper: Wrapper, ...renderOptions }) };
}
