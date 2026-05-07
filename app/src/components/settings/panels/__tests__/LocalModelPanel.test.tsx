import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import {
  type CommandResponse,
  type ConfigSnapshot,
  isTauri,
  type LocalAiDownloadsProgress,
  type LocalAiStatus,
  openhumanGetConfig,
  openhumanLocalAiDownload,
  openhumanLocalAiDownloadAllAssets,
  openhumanLocalAiDownloadsProgress,
  openhumanLocalAiPresets,
  openhumanLocalAiStatus,
  openhumanUpdateLocalAiSettings,
  type PresetsResponse,
} from '../../../../utils/tauriCommands';
import LocalModelPanel from '../LocalModelPanel';

vi.mock('../../../../utils/tauriCommands', () => ({
  isTauri: vi.fn(() => true),
  openhumanGetConfig: vi.fn(),
  openhumanLocalAiDownload: vi.fn(),
  openhumanLocalAiDownloadAllAssets: vi.fn(),
  openhumanLocalAiDownloadsProgress: vi.fn(),
  openhumanLocalAiPresets: vi.fn(),
  openhumanLocalAiStatus: vi.fn(),
  openhumanUpdateLocalAiSettings: vi.fn(),
}));

interface UsageFlags {
  runtime_enabled: boolean;
  embeddings: boolean;
  heartbeat: boolean;
  learning_reflection: boolean;
  subconscious: boolean;
}

const defaultUsage: UsageFlags = {
  runtime_enabled: false,
  embeddings: false,
  heartbeat: false,
  learning_reflection: false,
  subconscious: false,
};

const makeSnapshot = (flags: UsageFlags): CommandResponse<ConfigSnapshot> => ({
  result: {
    config: {
      local_ai: {
        runtime_enabled: flags.runtime_enabled,
        usage: {
          embeddings: flags.embeddings,
          heartbeat: flags.heartbeat,
          learning_reflection: flags.learning_reflection,
          subconscious: flags.subconscious,
        },
      },
    },
    workspace_dir: '/tmp/openhuman-test',
    config_path: '/tmp/openhuman-test/config.toml',
  },
  logs: [],
});

const idleStatus: LocalAiStatus = {
  state: 'idle',
  installed: false,
  download_progress: null,
  downloaded_bytes: null,
  total_bytes: null,
  download_speed_bps: null,
  eta_seconds: null,
  message: null,
  selected_tier: null,
} as unknown as LocalAiStatus;

const idleDownloads: LocalAiDownloadsProgress = {
  state: 'idle',
  progress: null,
  downloaded_bytes: null,
  total_bytes: null,
  speed_bps: null,
  eta_seconds: null,
} as unknown as LocalAiDownloadsProgress;

const presets: PresetsResponse = {
  presets: [],
  selected_tier: null,
  detected_ram_bytes: 16 * 1024 * 1024 * 1024,
  local_ai_enabled: false,
} as unknown as PresetsResponse;

describe('LocalModelPanel — usage flags', () => {
  let runtime: UsageFlags;

  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(isTauri).mockReturnValue(true);
    runtime = { ...defaultUsage };

    vi.mocked(openhumanLocalAiStatus).mockResolvedValue({ result: idleStatus, logs: [] });
    vi.mocked(openhumanLocalAiDownloadsProgress).mockResolvedValue({
      result: idleDownloads,
      logs: [],
    });
    vi.mocked(openhumanLocalAiPresets).mockResolvedValue(presets);
    vi.mocked(openhumanLocalAiDownload).mockResolvedValue({ result: idleStatus, logs: [] });
    vi.mocked(openhumanLocalAiDownloadAllAssets).mockResolvedValue({
      result: idleDownloads,
      logs: [],
    });

    vi.mocked(openhumanGetConfig).mockImplementation(async () => makeSnapshot(runtime));
    vi.mocked(openhumanUpdateLocalAiSettings).mockImplementation(async patch => {
      runtime = {
        runtime_enabled: patch.runtime_enabled ?? runtime.runtime_enabled,
        embeddings: patch.usage_embeddings ?? runtime.embeddings,
        heartbeat: patch.usage_heartbeat ?? runtime.heartbeat,
        learning_reflection: patch.usage_learning_reflection ?? runtime.learning_reflection,
        subconscious: patch.usage_subconscious ?? runtime.subconscious,
      };
      return makeSnapshot(runtime);
    });
  });

  it('renders all five usage toggles with sub-flags disabled when runtime is off', async () => {
    renderWithProviders(<LocalModelPanel />, { initialEntries: ['/settings/local-model'] });

    await screen.findByText('Enable local AI runtime');
    expect(screen.getByText('Embeddings')).toBeInTheDocument();
    expect(screen.getByText('Heartbeat')).toBeInTheDocument();
    expect(screen.getByText('Learning / reflection')).toBeInTheDocument();
    expect(screen.getByText('Subconscious')).toBeInTheDocument();

    // The four sub-flag inputs should be disabled while runtime is off
    const checkboxes = screen.getAllByRole('checkbox');
    const masterIdx = checkboxes.findIndex(cb =>
      cb.closest('label')?.textContent?.includes('Enable local AI runtime')
    );
    expect(masterIdx).toBeGreaterThanOrEqual(0);
    const subFlags = checkboxes.filter((_, i) => i !== masterIdx);
    for (const cb of subFlags) {
      expect(cb).toBeDisabled();
    }
  });

  it('persists master toggle change via openhumanUpdateLocalAiSettings', async () => {
    renderWithProviders(<LocalModelPanel />, { initialEntries: ['/settings/local-model'] });

    const masterLabel = await screen.findByText('Enable local AI runtime');
    const master = masterLabel.closest('label')?.querySelector('input[type="checkbox"]');
    expect(master).toBeTruthy();
    fireEvent.click(master as HTMLInputElement);

    await waitFor(() => {
      expect(openhumanUpdateLocalAiSettings).toHaveBeenCalledWith({ runtime_enabled: true });
    });
  });

  it('surfaces an error when the initial config load fails', async () => {
    vi.mocked(openhumanGetConfig).mockRejectedValueOnce(new Error('boom: get_config'));
    renderWithProviders(<LocalModelPanel />, { initialEntries: ['/settings/local-model'] });
    await screen.findByText('boom: get_config');
  });

  it('rolls state back and shows error when save fails', async () => {
    runtime.runtime_enabled = true;
    vi.mocked(openhumanUpdateLocalAiSettings).mockRejectedValueOnce(new Error('save: forbidden'));
    // Initial load succeeds; the reload triggered after a save error fails
    // too, so the error message is not immediately cleared by a successful
    // refetch. This exercises the catch arm in `updateUsage`.
    vi.mocked(openhumanGetConfig).mockImplementationOnce(async () => makeSnapshot(runtime));
    vi.mocked(openhumanGetConfig).mockRejectedValueOnce(new Error('save: forbidden'));
    renderWithProviders(<LocalModelPanel />, { initialEntries: ['/settings/local-model'] });

    const heartbeatLabel = await screen.findByText('Heartbeat');
    const cb = heartbeatLabel.closest('label')?.querySelector('input[type="checkbox"]');
    fireEvent.click(cb as HTMLInputElement);

    await waitFor(() => {
      expect(openhumanUpdateLocalAiSettings).toHaveBeenCalledWith({ usage_heartbeat: true });
    });
    await screen.findByText('save: forbidden');
  });

  it('persists a sub-flag toggle once master is enabled', async () => {
    runtime.runtime_enabled = true;
    renderWithProviders(<LocalModelPanel />, { initialEntries: ['/settings/local-model'] });

    const embeddingsLabel = await screen.findByText('Embeddings');
    const checkbox = embeddingsLabel.closest('label')?.querySelector('input[type="checkbox"]');
    expect(checkbox).toBeTruthy();
    expect(checkbox as HTMLInputElement).not.toBeDisabled();
    fireEvent.click(checkbox as HTMLInputElement);

    await waitFor(() => {
      expect(openhumanUpdateLocalAiSettings).toHaveBeenCalledWith({ usage_embeddings: true });
    });
  });
});
