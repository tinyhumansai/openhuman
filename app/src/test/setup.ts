/**
 * Global test setup for Vitest.
 *
 * - Extends expect with @testing-library/jest-dom matchers
 * - Starts local HTTP mock backend for API mocking
 * - Silences console output during tests (unless DEBUG_TESTS=1)
 * - Mocks Tauri-specific modules that aren't available in test env
 * - Resets rate limiter module-level state between tests
 */
import '@testing-library/jest-dom/vitest';
import { cleanup } from '@testing-library/react';
import type React from 'react';
import { afterAll, afterEach, beforeAll, beforeEach, vi } from 'vitest';

// @ts-ignore - test-only JS module outside app/src
import {
  clearRequestLog,
  resetMockBehavior,
  startMockServer,
  stopMockServer,
} from '../../../scripts/mock-api-core.mjs';

// Mock import.meta.env defaults for tests
vi.stubEnv('DEV', true);
vi.stubEnv('MODE', 'test');

// Mock Tauri APIs (not available in test env)
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(), isTauri: vi.fn(() => false) }));

vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(), emit: vi.fn() }));

vi.mock('@tauri-apps/plugin-deep-link', () => ({ onOpenUrl: vi.fn(), getCurrent: vi.fn() }));

vi.mock('@tauri-apps/plugin-opener', () => ({ open: vi.fn() }));

vi.mock('@tauri-apps/plugin-os', () => ({ platform: vi.fn().mockResolvedValue('macos') }));

// Mock tauriCommands to prevent Tauri API calls in tests
vi.mock('../utils/tauriCommands', () => ({
  isTauri: vi.fn(() => false),
  storeSession: vi.fn().mockResolvedValue(undefined),
  getSessionToken: vi.fn().mockResolvedValue(null),
  getAuthState: vi.fn().mockResolvedValue({ is_authenticated: false }),
  logout: vi.fn().mockResolvedValue(undefined),
  syncMemoryClientToken: vi.fn().mockResolvedValue(undefined),
  openhumanServiceInstall: vi.fn().mockResolvedValue({ result: { state: 'Running' }, logs: [] }),
  openhumanServiceStart: vi.fn().mockResolvedValue({ result: { state: 'Running' }, logs: [] }),
  openhumanServiceStop: vi.fn().mockResolvedValue({ result: { state: 'Stopped' }, logs: [] }),
  openhumanServiceStatus: vi.fn().mockResolvedValue({ result: { state: 'Running' }, logs: [] }),
  openhumanServiceUninstall: vi
    .fn()
    .mockResolvedValue({ result: { state: 'NotInstalled' }, logs: [] }),
  openhumanAgentServerStatus: vi.fn().mockResolvedValue({ result: { running: true }, logs: [] }),
  exchangeToken: vi.fn(),
  invoke: vi.fn(),
}));

// Mock the config module
vi.mock('../utils/config', () => ({
  TELEGRAM_BOT_USERNAME: 'test_bot',
  TELEGRAM_BOT_ID: '12345',
  IS_DEV: true,
  SKILLS_GITHUB_REPO: 'test/skills',
}));

vi.mock('../services/backendUrl', () => ({
  getBackendUrl: vi.fn().mockResolvedValue('http://localhost:5005'),
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

// Shared mock API server lifecycle for unit tests (default)
beforeAll(async () => {
  await startMockServer(5005);
});
afterEach(() => {
  clearRequestLog();
  cleanup();
});
afterAll(async () => {
  await stopMockServer();
});

// Reset rate limiter per-request counter before each test.
beforeEach(async () => {
  resetMockBehavior();
  try {
    const { resetRequestCallCount } = await import('../lib/mcp/rateLimiter');
    if (typeof resetRequestCallCount === 'function') {
      resetRequestCallCount();
    }
  } catch {
    // Module may be fully mocked in some test files — safe to skip
  }
});
