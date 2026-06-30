import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import SuperContextToggle from '../SuperContextToggle';

// `vi.mock` factories are hoisted above the static imports, so any module-level
// mock fns they reference must be created with `vi.hoisted` (hoisted alongside
// them) — a plain `const` would be in the TDZ when the factory runs and could
// throw on import. See CodeRabbit/Codex review on PR #4085.
const { isTauriMock, getMock, setMock } = vi.hoisted(() => ({
  isTauriMock: vi.fn(() => true),
  getMock: vi.fn(),
  setMock: vi.fn(),
}));

vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));
vi.mock('../../../utils/tauriCommands/common', () => ({ isTauri: () => isTauriMock() }));
vi.mock('../../../utils/tauriCommands/config', () => ({
  openhumanGetSuperContextEnabled: () => getMock(),
  openhumanSetSuperContextEnabled: (value: boolean) => setMock(value),
}));

beforeEach(() => {
  isTauriMock.mockReturnValue(true);
  getMock.mockReset();
  setMock.mockReset();
});

describe('<SuperContextToggle />', () => {
  it('loads the persisted flag and reflects it as the switch state', async () => {
    getMock.mockResolvedValue({ result: true, logs: [] });

    render(<SuperContextToggle />);

    const sw = screen.getByTestId('super-context-toggle');
    await waitFor(() => expect(sw).toHaveAttribute('aria-checked', 'true'));
    expect(getMock).toHaveBeenCalledTimes(1);
  });

  it('persists the new value and optimistically updates on toggle', async () => {
    getMock.mockResolvedValue({ result: false, logs: [] });
    setMock.mockResolvedValue({ result: true, logs: [] });

    render(<SuperContextToggle />);
    const sw = screen.getByTestId('super-context-toggle');
    // Wait for the initial read to enable the switch.
    await waitFor(() => expect(sw).not.toBeDisabled());

    fireEvent.click(sw);

    // Optimistic flip is immediate; persistence is called with the new value.
    await waitFor(() => expect(sw).toHaveAttribute('aria-checked', 'true'));
    expect(setMock).toHaveBeenCalledWith(true);
  });

  it('rolls back the optimistic flip when persistence fails', async () => {
    getMock.mockResolvedValue({ result: false, logs: [] });
    setMock.mockRejectedValue(new Error('rpc down'));

    render(<SuperContextToggle />);
    const sw = screen.getByTestId('super-context-toggle');
    await waitFor(() => expect(sw).not.toBeDisabled());

    fireEvent.click(sw);

    // Ends back at false after the rejected write.
    await waitFor(() => expect(sw).toHaveAttribute('aria-checked', 'false'));
    expect(setMock).toHaveBeenCalledWith(true);
  });

  it('does not call the core RPC when running outside Tauri', () => {
    isTauriMock.mockReturnValue(false);

    render(<SuperContextToggle />);

    expect(getMock).not.toHaveBeenCalled();
    const sw = screen.getByTestId('super-context-toggle');
    // Treated as loaded (enabled) immediately, default-off.
    expect(sw).toHaveAttribute('aria-checked', 'false');
  });
});
