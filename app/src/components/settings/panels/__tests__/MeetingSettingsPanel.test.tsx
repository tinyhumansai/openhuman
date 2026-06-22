/**
 * Meeting Assistant settings panel (issue #3511). Verifies each radio/toggle
 * reads its initial value from `openhumanGetMeetSettings` and writes via
 * `openhumanUpdateMeetSettings`.
 */
import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import {
  isTauri,
  type MeetSettings,
  openhumanGetMeetSettings,
  openhumanUpdateMeetSettings,
} from '../../../../utils/tauriCommands';
import MeetingSettingsPanel from '../MeetingSettingsPanel';

const meetSettings = (overrides: Partial<MeetSettings> = {}): MeetSettings => ({
  auto_orchestrator_handoff: false,
  auto_join_policy: 'ask_each_time',
  auto_summarize_policy: 'ask',
  listen_only_default: true,
  ingest_backend_transcripts: false,
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
    openhumanGetMeetSettings: vi.fn(),
    openhumanUpdateMeetSettings: vi.fn(),
  };
});

const mockGet = vi.mocked(openhumanGetMeetSettings);
const mockUpdate = vi.mocked(openhumanUpdateMeetSettings);

describe('MeetingSettingsPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(isTauri).mockReturnValue(true);
    mockGet.mockResolvedValue({ result: meetSettings(), logs: [] });
    mockUpdate.mockResolvedValue({ result: {} as never, logs: [] });
  });

  it('loads settings on mount', async () => {
    renderWithProviders(<MeetingSettingsPanel />);
    await waitFor(() => expect(mockGet).toHaveBeenCalledTimes(1));
  });

  it('renders the auto-join select with the current value', async () => {
    renderWithProviders(<MeetingSettingsPanel />);
    const select = await screen.findByRole('combobox', { name: /auto-join policy/i });
    expect(select).toHaveValue('ask_each_time');
  });

  it('changing auto-join persists the selection', async () => {
    renderWithProviders(<MeetingSettingsPanel />);
    const select = await screen.findByRole('combobox', { name: /auto-join policy/i });
    fireEvent.change(select, { target: { value: 'always' } });
    await waitFor(() =>
      expect(mockUpdate).toHaveBeenCalledWith(
        expect.objectContaining({ auto_join_policy: 'always' })
      )
    );
  });

  it('renders the auto-summarize select with the current value', async () => {
    renderWithProviders(<MeetingSettingsPanel />);
    const select = await screen.findByRole('combobox', { name: /post-call summary/i });
    expect(select).toHaveValue('ask');
  });

  it('changing auto-summarize persists the selection', async () => {
    renderWithProviders(<MeetingSettingsPanel />);
    const select = await screen.findByRole('combobox', { name: /post-call summary/i });
    fireEvent.change(select, { target: { value: 'never' } });
    await waitFor(() =>
      expect(mockUpdate).toHaveBeenCalledWith(
        expect.objectContaining({ auto_summarize_policy: 'never' })
      )
    );
  });

  it('toggling listen-only persists the change', async () => {
    renderWithProviders(<MeetingSettingsPanel />);
    const toggle = await screen.findByRole('switch', { name: /listen-only mode/i });
    fireEvent.click(toggle);
    await waitFor(() =>
      expect(mockUpdate).toHaveBeenCalledWith(
        expect.objectContaining({ listen_only_default: false })
      )
    );
  });

  it('toggling transcript ingestion persists the change', async () => {
    renderWithProviders(<MeetingSettingsPanel />);
    const toggle = await screen.findByRole('switch', { name: /ingest backend transcripts/i });
    fireEvent.click(toggle);
    await waitFor(() =>
      expect(mockUpdate).toHaveBeenCalledWith(
        expect.objectContaining({ ingest_backend_transcripts: true })
      )
    );
  });

  it('shows desktop-only message when not in Tauri', async () => {
    vi.mocked(isTauri).mockReturnValue(false);
    renderWithProviders(<MeetingSettingsPanel />);
    expect(await screen.findByText(/available on desktop only/i)).toBeInTheDocument();
    expect(mockGet).not.toHaveBeenCalled();
  });

  it('shows an error when settings fail to load', async () => {
    mockGet.mockRejectedValue(new Error('RPC timeout'));
    renderWithProviders(<MeetingSettingsPanel />);
    expect(await screen.findByText('RPC timeout')).toBeInTheDocument();
  });

  it('shows an error note when persist fails', async () => {
    mockUpdate.mockRejectedValue(new Error('Save failed'));
    renderWithProviders(<MeetingSettingsPanel />);
    const select = await screen.findByRole('combobox', { name: /auto-join policy/i });
    fireEvent.change(select, { target: { value: 'never' } });
    expect(await screen.findByText('Save failed')).toBeInTheDocument();
  });

  it('shows "Saved" note after a successful persist', async () => {
    renderWithProviders(<MeetingSettingsPanel />);
    const select = await screen.findByRole('combobox', { name: /auto-join policy/i });
    fireEvent.change(select, { target: { value: 'always' } });
    expect(await screen.findByText('Saved')).toBeInTheDocument();
  });

  it('rolls back auto-join select to previous value when persist fails', async () => {
    mockUpdate.mockRejectedValue(new Error('Save failed'));
    renderWithProviders(<MeetingSettingsPanel />);
    const select = await screen.findByRole('combobox', { name: /auto-join policy/i });
    expect(select).toHaveValue('ask_each_time');
    fireEvent.change(select, { target: { value: 'never' } });
    await screen.findByText('Save failed');
    expect(select).toHaveValue('ask_each_time');
  });

  it('rolls back auto-summarize select to previous value when persist fails', async () => {
    mockUpdate.mockRejectedValue(new Error('Save failed'));
    renderWithProviders(<MeetingSettingsPanel />);
    const select = await screen.findByRole('combobox', { name: /post-call summary/i });
    expect(select).toHaveValue('ask');
    fireEvent.change(select, { target: { value: 'never' } });
    await screen.findByText('Save failed');
    expect(select).toHaveValue('ask');
  });

  it('rolls back listen-only toggle to previous value when persist fails', async () => {
    mockUpdate.mockRejectedValue(new Error('Save failed'));
    renderWithProviders(<MeetingSettingsPanel />);
    const toggle = await screen.findByRole('switch', { name: /listen-only mode/i });
    expect(toggle).toHaveAttribute('aria-checked', 'true');
    fireEvent.click(toggle);
    await screen.findByText('Save failed');
    expect(toggle).toHaveAttribute('aria-checked', 'true');
  });

  it('rolls back transcript ingestion toggle to previous value when persist fails', async () => {
    mockUpdate.mockRejectedValue(new Error('Save failed'));
    renderWithProviders(<MeetingSettingsPanel />);
    const toggle = await screen.findByRole('switch', { name: /ingest backend transcripts/i });
    expect(toggle).toHaveAttribute('aria-checked', 'false');
    fireEvent.click(toggle);
    await screen.findByText('Save failed');
    expect(toggle).toHaveAttribute('aria-checked', 'false');
  });
});
