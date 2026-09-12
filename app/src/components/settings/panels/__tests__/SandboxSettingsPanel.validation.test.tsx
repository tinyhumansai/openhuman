/**
 * SandboxSettingsPanel — the input the blur handlers must REFUSE, the null
 * limits on load, and the persist sequence guard.
 *
 * `SandboxSettingsPanel.test.tsx` (sibling) covers each field's happy path and
 * the explicit clear-to-null. What it never does is hand a field something
 * invalid: `handleMemoryBlur` (`SandboxSettingsPanel.tsx:113-123`) and
 * `handleCpuBlur` (`:125-135`) both drop non-numeric and non-positive input on
 * the floor, and `handleDockerImageBlur` (`:107-111`) drops a blank image.
 * Those three guards were entirely unexercised — branch coverage 71.4%.
 *
 * The guards matter: a `0` or negative container limit that reached the core
 * would be a Docker run argument, and a blank image would replace a working one.
 *
 * Separate file rather than an edit to the sibling: three of us are in this
 * directory.
 */
import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import {
  isTauri,
  openhumanGetSandboxSettings,
  openhumanUpdateSandboxSettings,
  type SandboxSettings,
} from '../../../../utils/tauriCommands';
import SandboxSettingsPanel from '../SandboxSettingsPanel';

const sandboxSettings = (overrides: Partial<SandboxSettings> = {}): SandboxSettings => ({
  enabled: true,
  backend: 'auto',
  docker_image: 'alpine:3.20',
  docker_memory_limit_mb: 512,
  docker_cpu_limit: 1.0,
  docker_available: true,
  detected_backend: 'seatbelt',
  env_passthrough: ['PATH', 'HOME', 'TERM'],
  ...overrides,
});

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateBack: vi.fn(),
    navigateToSettings: vi.fn(),
    breadcrumbs: [],
  }),
}));

vi.mock('../../../../utils/tauriCommands', async () => {
  const actual = await vi.importActual<typeof import('../../../../utils/tauriCommands')>(
    '../../../../utils/tauriCommands'
  );
  return {
    ...actual,
    isTauri: vi.fn(() => true),
    openhumanGetSandboxSettings: vi.fn(),
    openhumanUpdateSandboxSettings: vi.fn(),
  };
});

const mockGet = vi.mocked(openhumanGetSandboxSettings);
const mockUpdate = vi.mocked(openhumanUpdateSandboxSettings);
const mockIsTauri = vi.mocked(isTauri);

async function renderLoaded(overrides: Partial<SandboxSettings> = {}) {
  mockGet.mockResolvedValue({ result: sandboxSettings(overrides), logs: [] });
  renderWithProviders(<SandboxSettingsPanel />);
  await waitFor(() => expect(mockGet).toHaveBeenCalled());
}

/**
 * Let any queued persist actually run before asserting it did NOT happen.
 *
 * `await waitFor(() => expect(mockGet).toHaveBeenCalled())` is NOT a readiness
 * signal here: `mockGet` fires on mount, so the condition is already true and
 * `waitFor` resolves on its first tick — before a blur handler's promise chain
 * has had a chance to reach `mockUpdate`. The negative assertion that follows
 * would then pass whether or not the guard exists. Two macrotasks is what a
 * `void persist(...)` needs to get from the handler to the mock.
 */
async function flushPersist() {
  await act(async () => {
    await new Promise(resolve => setTimeout(resolve, 0));
    await new Promise(resolve => setTimeout(resolve, 0));
  });
}

/** Blur a field after typing `value`, then report what (if anything) persisted. */
function blurWith(input: HTMLElement, value: string) {
  fireEvent.change(input, { target: { value } });
  fireEvent.blur(input);
}

const memoryInput = () => screen.getByDisplayValue('512');
const cpuInput = () => screen.getByDisplayValue('1');
const imageInput = () => screen.getByDisplayValue('alpine:3.20');

beforeEach(() => {
  vi.clearAllMocks();
  mockIsTauri.mockReturnValue(true);
  mockUpdate.mockResolvedValue(undefined as never);
});

describe('SandboxSettingsPanel — memory limit validation', () => {
  // Only values a `type="number"` field can actually hold. Non-numeric text
  // ('abc', '!!') is unreachable here: jsdom, like a browser, reports an empty
  // value for it, which lands on the clear-to-null branch instead. The
  // `!isNaN(parsed)` half of the guard is therefore dead through the UI — see
  // the note in the bug list.
  it.each(['0', '-1', '-512'])('refuses to persist %s', async value => {
    await renderLoaded();
    blurWith(memoryInput(), value);

    await flushPersist();
    expect(mockUpdate).not.toHaveBeenCalled();
  });

  it('persists a positive integer', async () => {
    await renderLoaded();
    blurWith(memoryInput(), '2048');

    await waitFor(() => expect(mockUpdate).toHaveBeenCalledWith({ docker_memory_limit_mb: 2048 }));
  });

  it('persists null when the field is cleared to whitespace', async () => {
    await renderLoaded();
    blurWith(memoryInput(), '   ');

    await waitFor(() => expect(mockUpdate).toHaveBeenCalledWith({ docker_memory_limit_mb: null }));
  });
});

describe('SandboxSettingsPanel — CPU limit validation', () => {
  it.each(['0', '-0.5'])('refuses to persist %s', async value => {
    await renderLoaded();
    blurWith(cpuInput(), value);

    await flushPersist();
    expect(mockUpdate).not.toHaveBeenCalled();
  });

  it('persists a fractional limit', async () => {
    await renderLoaded();
    blurWith(cpuInput(), '0.25');

    await waitFor(() => expect(mockUpdate).toHaveBeenCalledWith({ docker_cpu_limit: 0.25 }));
  });

  it('persists null when the field is cleared to whitespace', async () => {
    await renderLoaded();
    blurWith(cpuInput(), '  ');

    await waitFor(() => expect(mockUpdate).toHaveBeenCalledWith({ docker_cpu_limit: null }));
  });
});

describe('SandboxSettingsPanel — docker image validation', () => {
  it('refuses to persist a blank image, keeping the configured one', async () => {
    await renderLoaded();
    blurWith(imageInput(), '   ');

    await flushPersist();
    expect(mockUpdate).not.toHaveBeenCalled();
  });

  it('trims the image before persisting', async () => {
    await renderLoaded();
    blurWith(imageInput(), '  ubuntu:24.04  ');

    await waitFor(() => expect(mockUpdate).toHaveBeenCalledWith({ docker_image: 'ubuntu:24.04' }));
  });
});

describe('SandboxSettingsPanel — load shape', () => {
  it('renders empty limit fields when the core reports no limits', async () => {
    await renderLoaded({ docker_memory_limit_mb: null, docker_cpu_limit: null });

    // Wait for a POSITIVE loaded signal first — the image field is populated
    // from the same response. Waiting on the absence of '512' would pass while
    // the panel was still empty because nothing had loaded at all.
    expect(await screen.findByDisplayValue('alpine:3.20')).toBeInTheDocument();
    expect(screen.queryByDisplayValue('512')).not.toBeInTheDocument();
    expect(screen.queryByDisplayValue('1')).not.toBeInTheDocument();
  });

  it('does not call the core at all off-Tauri', async () => {
    mockIsTauri.mockReturnValue(false);
    renderWithProviders(<SandboxSettingsPanel />);

    await waitFor(() => expect(screen.getByText(/desktop app/i)).toBeInTheDocument());
    expect(mockGet).not.toHaveBeenCalled();
  });
});

describe('SandboxSettingsPanel — persist sequence guard', () => {
  /** Let a settled promise's continuation and the resulting render run. A bare
   *  `waitFor(() => expect(x).not.toBeInTheDocument())` passes on its first
   *  tick, before the rejection is handled, and would hold with the guard gone. */
  async function flush() {
    await waitFor(() => expect(mockUpdate).toHaveBeenCalled());
    await new Promise(resolve => setTimeout(resolve, 0));
    await new Promise(resolve => setTimeout(resolve, 0));
  }

  it('drops a stale persist failure that lands after a newer persist', async () => {
    let rejectFirst!: (e: unknown) => void;
    mockUpdate.mockReturnValueOnce(
      new Promise((_res, rej) => {
        rejectFirst = rej;
      }) as never
    );
    mockUpdate.mockResolvedValueOnce(undefined as never);

    await renderLoaded();
    blurWith(memoryInput(), '1024');
    blurWith(cpuInput(), '2');
    await waitFor(() => expect(mockUpdate).toHaveBeenCalledTimes(2));

    rejectFirst(new Error('stale sandbox failure'));
    await flush();

    expect(screen.queryByText(/stale sandbox failure/)).not.toBeInTheDocument();
  });
});
