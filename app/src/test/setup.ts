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
import { afterAll, afterEach, beforeEach, vi } from 'vitest';

// @ts-ignore - test-only JS module outside app/src
import {
  clearRequestLog,
  resetMockBehavior,
  startMockServer,
  stopMockServer,
} from '../../../scripts/mock-api-core.mjs';

const DEFAULT_TEST_MOCK_API_PORT = 5005;

function readMockApiPort() {
  const rawPort = process.env.VITEST_MOCK_API_PORT ?? process.env.MOCK_API_PORT;
  const port = rawPort ? Number(rawPort) : DEFAULT_TEST_MOCK_API_PORT;
  return Number.isInteger(port) && port > 0 ? port : DEFAULT_TEST_MOCK_API_PORT;
}

const mockApiServer = await startMockServer(readMockApiPort(), { retryIfInUse: true });
const mockApiUrl = `http://localhost:${mockApiServer.port}`;
process.env.VITEST_MOCK_API_URL = mockApiUrl;
process.env.VITE_BACKEND_URL = mockApiUrl;

// Mock import.meta.env defaults for tests
vi.stubEnv('DEV', true);
vi.stubEnv('MODE', 'test');
vi.stubEnv('VITE_BACKEND_URL', mockApiUrl);

function createStorageMock(): Storage {
  const store = new Map<string, string>();
  return {
    get length() {
      return store.size;
    },
    clear() {
      store.clear();
    },
    getItem(key: string) {
      return store.has(key) ? store.get(key)! : null;
    },
    key(index: number) {
      return Array.from(store.keys())[index] ?? null;
    },
    removeItem(key: string) {
      store.delete(key);
    },
    setItem(key: string, value: string) {
      store.set(String(key), String(value));
    },
  };
}

function ensureStorage(name: 'localStorage' | 'sessionStorage') {
  const current = globalThis[name];
  if (
    current &&
    typeof current.getItem === 'function' &&
    typeof current.setItem === 'function' &&
    typeof current.removeItem === 'function' &&
    typeof current.clear === 'function'
  ) {
    return;
  }

  Object.defineProperty(globalThis, name, {
    value: createStorageMock(),
    configurable: true,
    writable: true,
  });
}

ensureStorage('localStorage');
ensureStorage('sessionStorage');

// Polyfill ResizeObserver for cmdk/Radix components in jsdom
if (typeof globalThis.ResizeObserver === 'undefined') {
  globalThis.ResizeObserver = class ResizeObserver {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}

// Polyfill scrollIntoView for cmdk in jsdom
if (typeof Element !== 'undefined' && !Element.prototype.scrollIntoView) {
  Element.prototype.scrollIntoView = function () {};
}

// The hardened `isTauri()` (in `utils/tauriCommands/common.ts`) checks both
// `coreIsTauri()` and `window.__TAURI_INTERNALS__.invoke`. Many existing test
// files mock `@tauri-apps/api/core::isTauri` to `true` to exercise the
// Tauri branch; without a matching IPC handle on `window` they would now
// regress to the non-Tauri path. Seed a no-op handle once globally so the
// IPC-readiness check passes by default. Tests that *want* the CEF gap
// behaviour can `delete window.__TAURI_INTERNALS__` in a `beforeEach`.
(
  window as unknown as { __TAURI_INTERNALS__: { invoke: () => Promise<unknown> } }
).__TAURI_INTERNALS__ = { invoke: vi.fn(() => Promise.resolve()) };

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
  openhumanUpdateMeetSettings: vi
    .fn()
    .mockResolvedValue({
      result: { config: {}, workspace_dir: '/tmp', config_path: '/tmp/cfg.toml' },
      logs: [],
    }),
  openhumanGetMeetSettings: vi
    .fn()
    .mockResolvedValue({ result: { auto_orchestrator_handoff: false }, logs: [] }),
  exchangeToken: vi.fn(),
  invoke: vi.fn(),
}));

// Mock the config module
vi.mock('../utils/config', () => ({
  CORE_RPC_URL: 'http://127.0.0.1:7788/rpc',
  CORE_RPC_TIMEOUT_MS: 30_000,
  IS_DEV: true,
  IS_PROD: false,
  DEV_FORCE_ONBOARDING: false,
  SKILLS_GITHUB_REPO: 'test/skills',
  SENTRY_DSN: undefined,
  BACKEND_URL: mockApiUrl,
  TELEGRAM_BOT_USERNAME: 'openhuman_bot',
  LATEST_APP_DOWNLOAD_URL: 'https://github.com/tinyhumansai/openhuman/releases/latest',
  APP_VERSION: '0.0.0-test',
  DEV_JWT_TOKEN: undefined,
  MASCOT_VOICE_ID: 'ljX1ZrXuDIIRVcmiVSyR',
}));

vi.mock('../services/backendUrl', () => ({
  getBackendUrl: vi.fn().mockImplementation(() => Promise.resolve(mockApiUrl)),
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
afterEach(() => {
  clearRequestLog();
  cleanup();
  // Re-seed the IPC handle after any test that may have deleted it
  // (e.g. tests exercising the CEF-gap branch of `isTauri()`). Without
  // this, sibling tests in the same jsdom worker would silently regress
  // to the non-Tauri path. Per graycyrus review on PR #1556.
  (
    window as unknown as { __TAURI_INTERNALS__: { invoke: () => Promise<unknown> } }
  ).__TAURI_INTERNALS__ = { invoke: vi.fn(() => Promise.resolve()) };
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
