/**
 * Global test setup for Vitest.
 *
 * - Extends expect with @testing-library/jest-dom matchers
 * - Sets up MSW server for API mocking
 * - Silences console output during tests (unless DEBUG_TESTS=1)
 * - Mocks Tauri-specific modules that aren't available in test env
 * - Resets rate limiter module-level state between tests
 */
import '@testing-library/jest-dom/vitest';
import { cleanup } from '@testing-library/react';
import type React from 'react';
import { afterAll, afterEach, beforeAll, beforeEach, vi } from 'vitest';

import { server } from './server';

// Mock import.meta.env defaults for tests
vi.stubEnv('VITE_BACKEND_URL', 'http://localhost:5005');
vi.stubEnv('DEV', true);
vi.stubEnv('MODE', 'test');

// Mock Tauri APIs (not available in test env)
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(), emit: vi.fn() }));

vi.mock('@tauri-apps/plugin-deep-link', () => ({ onOpenUrl: vi.fn(), getCurrent: vi.fn() }));

vi.mock('@tauri-apps/plugin-opener', () => ({ open: vi.fn() }));

vi.mock('@tauri-apps/plugin-os', () => ({ platform: vi.fn().mockResolvedValue('macos') }));

vi.mock('@tauri-apps/plugin-shell', () => ({ Command: vi.fn(), open: vi.fn() }));

// Mock tauriCommands to prevent Tauri API calls in tests
vi.mock('../utils/tauriCommands', () => ({
  isTauri: () => false,
  storeSession: vi.fn().mockResolvedValue(undefined),
  getAuthState: vi.fn().mockResolvedValue({ is_authenticated: false }),
  exchangeToken: vi.fn(),
  invoke: vi.fn(),
}));

// Mock the config module
vi.mock('../utils/config', () => ({
  BACKEND_URL: 'http://localhost:5005',
  TELEGRAM_BOT_USERNAME: 'test_bot',
  TELEGRAM_BOT_ID: '12345',
  IS_DEV: true,
  SKILLS_GITHUB_REPO: 'test/skills',
  DEV_AUTO_LOAD_SKILL: undefined,
}));

// Mock redux-persist to avoid CJS/ESM issues in vitest
vi.mock('redux-persist', async () => {
  const actual = await vi.importActual<Record<string, unknown>>('redux-persist');
  return {
    ...actual,
    // Override persistReducer to just return the base reducer
    persistReducer: (_config: unknown, reducer: (s: unknown, a: unknown) => unknown) => reducer,
    // Override persistStore to return a no-op persistor
    persistStore: () => ({
      subscribe: () => () => {},
      getState: () => ({}),
      dispatch: () => {},
      purge: () => Promise.resolve(),
      flush: () => Promise.resolve(),
      pause: () => {},
      persist: () => {},
    }),
  };
});

// Mock redux-persist integration
vi.mock('redux-persist/integration/react', () => ({
  PersistGate: ({
    children,
  }: {
    children: React.ReactNode;
    loading?: unknown;
    persistor?: unknown;
  }) => children,
}));

// Mock redux-logger to avoid noisy test output
vi.mock('redux-logger', () => ({
  createLogger: () => () => (next: (action: unknown) => unknown) => (action: unknown) =>
    next(action),
}));

// Mock Sentry
vi.mock('@sentry/react', () => ({
  init: vi.fn(),
  ErrorBoundary: ({
    children,
  }: {
    children: React.ReactNode;
    fallback?: unknown;
    onError?: unknown;
  }) => children,
  withScope: vi.fn(),
  captureException: vi.fn(),
  setTag: vi.fn(),
  setUser: vi.fn(),
}));

// Silence console during tests to keep output clean
if (!process.env.DEBUG_TESTS) {
  vi.spyOn(console, 'log').mockImplementation(() => {});
  vi.spyOn(console, 'warn').mockImplementation(() => {});
  vi.spyOn(console, 'error').mockImplementation(() => {});
}

// MSW server lifecycle
beforeAll(() => server.listen({ onUnhandledRequest: 'bypass' }));
afterEach(() => {
  server.resetHandlers();
  cleanup();
});
afterAll(() => server.close());

// Reset rate limiter per-request counter before each test.
beforeEach(async () => {
  try {
    const { resetRequestCallCount } = await import('../lib/mcp/rateLimiter');
    if (typeof resetRequestCallCount === 'function') {
      resetRequestCallCount();
    }
  } catch {
    // Module may be fully mocked in some test files — safe to skip
  }
});
