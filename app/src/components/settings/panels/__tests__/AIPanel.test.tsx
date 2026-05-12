import { fireEvent, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import {
  aiGetConfig,
  type AIPreview,
  aiRefreshConfig,
  type CommandResponse,
  type LocalAiStatus,
  openhumanLocalAiDownload,
  openhumanLocalAiStatus,
} from '../../../../utils/tauriCommands';
import AIPanel from '../AIPanel';

vi.mock('../../../../utils/tauriCommands', () => ({
  aiGetConfig: vi.fn(),
  aiRefreshConfig: vi.fn(),
  openhumanLocalAiDownload: vi.fn(),
  openhumanLocalAiStatus: vi.fn(),
}));

const aiPreview: AIPreview = {
  soul: {
    raw: '',
    name: 'OpenHuman',
    description: 'Test persona',
    personalityPreview: [],
    safetyRulesPreview: [],
    loadedAt: 1,
  },
  tools: { raw: '', totalTools: 0, activeSkills: 0, skillsPreview: [], loadedAt: 1 },
  metadata: {
    loadedAt: 1,
    loadingDuration: 1,
    hasFallbacks: false,
    sources: { soul: 'test', tools: 'test' },
    errors: [],
  },
};

const disabledStatus: LocalAiStatus = {
  state: 'disabled',
  model_id: 'local-v1',
} as unknown as LocalAiStatus;

describe('AIPanel local model runtime gate', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(aiGetConfig).mockResolvedValue(aiPreview);
    vi.mocked(aiRefreshConfig).mockResolvedValue(aiPreview);
    vi.mocked(openhumanLocalAiStatus).mockResolvedValue({
      result: disabledStatus,
      logs: [],
    } as CommandResponse<LocalAiStatus>);
    vi.mocked(openhumanLocalAiDownload).mockResolvedValue({
      result: disabledStatus,
      logs: [],
    } as CommandResponse<LocalAiStatus>);
  });

  it('does not retry downloads while local AI runtime is disabled', async () => {
    renderWithProviders(<AIPanel />, { initialEntries: ['/settings/ai'] });

    const retryButton = await screen.findByRole('button', { name: 'Retry Download' });
    expect(retryButton).toBeDisabled();
    fireEvent.click(retryButton);

    expect(openhumanLocalAiDownload).not.toHaveBeenCalled();
  });
});
