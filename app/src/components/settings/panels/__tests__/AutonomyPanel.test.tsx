import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import {
  type AutonomySettings,
  openhumanGetAutonomySettings,
  openhumanUpdateAutonomySettings,
} from '../../../../utils/tauriCommands/config';
import AutonomyRateLimitSection from '../AutonomyPanel';

// AutonomyRateLimitSection only reads/writes `max_actions_per_hour`, but the settings RPC
// now returns the full access-mode block — build a complete value so the mocks
// satisfy `AutonomySettings`.
const autonomy = (max_actions_per_hour: number): AutonomySettings => ({
  level: 'supervised',
  workspace_only: false,
  allowed_commands: [],
  forbidden_paths: [],
  trusted_roots: [],
  allow_tool_install: false,
  max_actions_per_hour,
  auto_approve: [],
});

vi.mock('../../../../utils/tauriCommands/config', async () => {
  const actual = await vi.importActual<typeof import('../../../../utils/tauriCommands/config')>(
    '../../../../utils/tauriCommands/config'
  );
  return {
    ...actual,
    openhumanGetAutonomySettings: vi.fn(),
    openhumanUpdateAutonomySettings: vi.fn(),
  };
});

const mockGet = vi.mocked(openhumanGetAutonomySettings);
const mockUpdate = vi.mocked(openhumanUpdateAutonomySettings);

describe('AutonomyRateLimitSection', () => {
  beforeEach(() => {
    mockGet.mockReset();
    mockUpdate.mockReset();
  });

  test('loads the current value on mount', async () => {
    mockGet.mockResolvedValue({ result: autonomy(250), logs: [] });
    renderWithProviders(<AutonomyRateLimitSection />, {
      initialEntries: ['/settings/agent-access'],
    });
    const input = (await screen.findByLabelText(/Max actions per hour/i)) as HTMLInputElement;
    await waitFor(() => expect(input).toHaveValue(250));
  });

  test('Save is disabled until the value changes', async () => {
    mockGet.mockResolvedValue({ result: autonomy(20), logs: [] });
    renderWithProviders(<AutonomyRateLimitSection />, {
      initialEntries: ['/settings/agent-access'],
    });
    const saveBtn = await screen.findByRole('button', { name: /^Save$/ });
    expect(saveBtn).toBeDisabled();

    const input = await screen.findByDisplayValue('20');
    fireEvent.change(input, { target: { value: '100' } });
    expect(saveBtn).not.toBeDisabled();
  });

  test('Save invokes the wrapper and shows confirmation', async () => {
    mockGet.mockResolvedValue({ result: autonomy(20), logs: [] });
    mockUpdate.mockResolvedValue({
      result: { config: {}, workspace_dir: '/tmp', config_path: '/tmp/cfg.toml' },
      logs: [],
    });
    renderWithProviders(<AutonomyRateLimitSection />, {
      initialEntries: ['/settings/agent-access'],
    });
    const input = await screen.findByDisplayValue('20');
    fireEvent.change(input, { target: { value: '300' } });
    fireEvent.click(screen.getByRole('button', { name: /^Save$/ }));
    await waitFor(() => expect(mockUpdate).toHaveBeenCalledWith({ max_actions_per_hour: 300 }));
    await screen.findByText(/Saved\./i);
  });

  test('shows inline validation when the value is out of range', async () => {
    mockGet.mockResolvedValue({ result: autonomy(20), logs: [] });
    renderWithProviders(<AutonomyRateLimitSection />, {
      initialEntries: ['/settings/agent-access'],
    });
    const input = await screen.findByDisplayValue('20');
    fireEvent.change(input, { target: { value: '0' } });
    await screen.findByText(/Must be a positive integer/i);
    expect(screen.getByRole('button', { name: /^Save$/ })).toBeDisabled();
  });

  // Note: '12abc' is omitted because <input type="number"> filters non-numeric
  // characters before React sees the change event — there's no way the panel
  // can receive that input through normal UI flow.
  test.each(['1.5', '1e2', '-5', '0.0'])('rejects non-integer input %s', async value => {
    mockGet.mockResolvedValue({ result: autonomy(20), logs: [] });
    renderWithProviders(<AutonomyRateLimitSection />, {
      initialEntries: ['/settings/agent-access'],
    });
    const input = await screen.findByDisplayValue('20');
    fireEvent.change(input, { target: { value } });
    await screen.findByText(/Must be a positive integer/i);
    expect(screen.getByRole('button', { name: /^Save$/ })).toBeDisabled();
  });

  test('surfaces RPC errors and reverts to the last committed value', async () => {
    mockGet.mockResolvedValue({ result: autonomy(50), logs: [] });
    mockUpdate.mockRejectedValue(new Error('disk full'));
    renderWithProviders(<AutonomyRateLimitSection />, {
      initialEntries: ['/settings/agent-access'],
    });
    const input = (await screen.findByDisplayValue('50')) as HTMLInputElement;
    fireEvent.change(input, { target: { value: '500' } });
    fireEvent.click(screen.getByRole('button', { name: /^Save$/ }));
    await screen.findByText(/Failed: disk full/);
    // Reverted to last committed value.
    expect(input).toHaveValue(50);
  });

  // ─── Preset buttons ───────────────────────────────────────────────────────

  test('clicking the 100 preset sets the draft to 100', async () => {
    mockGet.mockResolvedValue({ result: autonomy(20), logs: [] });
    renderWithProviders(<AutonomyRateLimitSection />, {
      initialEntries: ['/settings/agent-access'],
    });
    const input = (await screen.findByDisplayValue('20')) as HTMLInputElement;

    fireEvent.click(screen.getByRole('button', { name: '100' }));
    await waitFor(() => expect(input).toHaveValue(100));
    expect(screen.getByRole('button', { name: /^Save$/ })).not.toBeDisabled();
  });

  test('clicking the 500 preset sets the draft to 500', async () => {
    mockGet.mockResolvedValue({ result: autonomy(20), logs: [] });
    renderWithProviders(<AutonomyRateLimitSection />, {
      initialEntries: ['/settings/agent-access'],
    });
    const input = (await screen.findByDisplayValue('20')) as HTMLInputElement;

    fireEvent.click(screen.getByRole('button', { name: '500' }));
    await waitFor(() => expect(input).toHaveValue(500));
  });

  test('clicking the 1000 preset sets the draft to 1000', async () => {
    mockGet.mockResolvedValue({ result: autonomy(20), logs: [] });
    renderWithProviders(<AutonomyRateLimitSection />, {
      initialEntries: ['/settings/agent-access'],
    });
    const input = (await screen.findByDisplayValue('20')) as HTMLInputElement;

    fireEvent.click(screen.getByRole('button', { name: '1000' }));
    await waitFor(() => expect(input).toHaveValue(1000));
  });

  test('clicking Unlimited preset sets draft to UNLIMITED sentinel and shows note', async () => {
    mockGet.mockResolvedValue({ result: autonomy(20), logs: [] });
    renderWithProviders(<AutonomyRateLimitSection />, {
      initialEntries: ['/settings/agent-access'],
    });
    await screen.findByDisplayValue('20');

    // The Unlimited preset button uses i18n key autonomy.presetUnlimited
    const unlimitedBtn = screen.getByRole('button', { name: /unlimited/i });
    fireEvent.click(unlimitedBtn);

    // The input value should be set to the UNLIMITED sentinel (4294967295)
    const input = screen.getByRole('spinbutton') as HTMLInputElement;
    await waitFor(() => expect(Number(input.value)).toBe(4_294_967_295));
    // Save button enabled because value changed
    expect(screen.getByRole('button', { name: /^Save$/ })).not.toBeDisabled();
  });

  // ─── Status transitions on re-edit ───────────────────────────────────────

  test('editing the field after save clears the saved status', async () => {
    mockGet.mockResolvedValue({ result: autonomy(20), logs: [] });
    mockUpdate.mockResolvedValue({
      result: { config: {}, workspace_dir: '/tmp', config_path: '/tmp/cfg.toml' },
      logs: [],
    });
    renderWithProviders(<AutonomyRateLimitSection />, {
      initialEntries: ['/settings/agent-access'],
    });
    const input = await screen.findByDisplayValue('20');

    fireEvent.change(input, { target: { value: '300' } });
    fireEvent.click(screen.getByRole('button', { name: /^Save$/ }));
    await screen.findByText(/Saved\./i);

    // Now edit again — saved note disappears and Save re-enables
    fireEvent.change(input, { target: { value: '400' } });
    await waitFor(() => expect(screen.queryByText(/Saved\./i)).not.toBeInTheDocument());
    expect(screen.getByRole('button', { name: /^Save$/ })).not.toBeDisabled();
  });

  test('editing the field after error clears the error status', async () => {
    mockGet.mockResolvedValue({ result: autonomy(50), logs: [] });
    mockUpdate.mockRejectedValue(new Error('disk full'));
    renderWithProviders(<AutonomyRateLimitSection />, {
      initialEntries: ['/settings/agent-access'],
    });
    const input = await screen.findByDisplayValue('50');

    fireEvent.change(input, { target: { value: '500' } });
    fireEvent.click(screen.getByRole('button', { name: /^Save$/ }));
    await screen.findByText(/Failed: disk full/);

    // Re-edit clears the error
    fireEvent.change(input, { target: { value: '200' } });
    await waitFor(() => expect(screen.queryByText(/Failed: disk full/)).not.toBeInTheDocument());
  });
});
