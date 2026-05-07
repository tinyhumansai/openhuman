/**
 * Tests for the Settings → Memory Data panel.
 *
 * Verifies that all four memory-window preset buttons render, the memory
 * sources section is present, and that a sync-connection error does not
 * hide or disable the memory-window controls.
 */
import { screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import MemoryDataPanel from '../MemoryDataPanel';

// ── Mocks ────────────────────────────────────────────────────────────────────

const hoisted = vi.hoisted(() => ({
  mockGetConfig: vi.fn(),
  mockUpdateMemorySettings: vi.fn(),
  mockIsTauri: vi.fn(() => false),
  mockMemoryTreeListChunks: vi.fn(),
  mockMemoryTreeListSources: vi.fn(),
  mockMemoryTreeTopEntities: vi.fn(),
  mockMemoryTreeEntityIndexFor: vi.fn(),
  mockMemoryTreeChunkScore: vi.fn(),
  mockMemoryTreeChunksForEntity: vi.fn(),
  mockStatusList: vi.fn(),
}));

vi.mock('../../../../utils/tauriCommands', () => ({
  isTauri: hoisted.mockIsTauri,
  openhumanGetConfig: hoisted.mockGetConfig,
  openhumanUpdateMemorySettings: hoisted.mockUpdateMemorySettings,
  MEMORY_CONTEXT_WINDOWS: ['minimal', 'balanced', 'extended', 'maximum'],
  memoryTreeListChunks: hoisted.mockMemoryTreeListChunks,
  memoryTreeListSources: hoisted.mockMemoryTreeListSources,
  memoryTreeTopEntities: hoisted.mockMemoryTreeTopEntities,
  memoryTreeEntityIndexFor: hoisted.mockMemoryTreeEntityIndexFor,
  memoryTreeChunkScore: hoisted.mockMemoryTreeChunkScore,
  memoryTreeChunksForEntity: hoisted.mockMemoryTreeChunksForEntity,
}));

vi.mock('../../../../services/memorySyncService', async () => {
  const actual = await vi.importActual<typeof import('../../../../services/memorySyncService')>(
    '../../../../services/memorySyncService'
  );
  return {
    ...actual,
    memorySyncStatusList: (...args: unknown[]) => hoisted.mockStatusList(...args),
  };
});

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({ navigateBack: vi.fn(), breadcrumbs: [] }),
}));

// ── Helpers ───────────────────────────────────────────────────────────────────

const resolveConfigWith = (memory_window = 'balanced') => {
  hoisted.mockGetConfig.mockResolvedValue({
    result: {
      config: { agent: { memory_window } },
      workspace_dir: '/tmp/ws',
      config_path: '/tmp/cfg.toml',
    },
  });
};

// ── Tests ─────────────────────────────────────────────────────────────────────

describe('MemoryDataPanel', () => {
  beforeEach(() => {
    hoisted.mockGetConfig.mockReset();
    hoisted.mockUpdateMemorySettings.mockReset();
    hoisted.mockIsTauri.mockReturnValue(false);
    hoisted.mockStatusList.mockReset();
    hoisted.mockMemoryTreeListChunks.mockReset();
    hoisted.mockMemoryTreeListSources.mockReset();
    hoisted.mockMemoryTreeTopEntities.mockReset();
    hoisted.mockMemoryTreeEntityIndexFor.mockReset();
    hoisted.mockMemoryTreeChunkScore.mockReset();
    hoisted.mockMemoryTreeChunksForEntity.mockReset();

    // Default: no sources yet, no errors
    hoisted.mockStatusList.mockResolvedValue([]);
    hoisted.mockMemoryTreeListChunks.mockResolvedValue({ chunks: [], total: 0, cursor: null });
    hoisted.mockMemoryTreeListSources.mockResolvedValue([]);
    hoisted.mockMemoryTreeTopEntities.mockResolvedValue([]);
  });

  it('renders all four preset buttons', async () => {
    resolveConfigWith('balanced');
    renderWithProviders(<MemoryDataPanel />);

    for (const label of ['Minimal', 'Balanced', 'Extended', 'Maximum']) {
      expect(screen.getByText(label)).toBeInTheDocument();
    }
  });

  it('renders the memory sources section', async () => {
    resolveConfigWith('balanced');
    renderWithProviders(<MemoryDataPanel />);

    await waitFor(() => {
      expect(screen.getByTestId('memory-sources')).toBeInTheDocument();
    });
  });

  it('keeps all preset buttons accessible when sync connections returns an error', async () => {
    resolveConfigWith('balanced');
    hoisted.mockStatusList.mockRejectedValue(new Error('network timeout'));

    renderWithProviders(<MemoryDataPanel />);

    // Wait for the error state to appear in the sync connections section
    await waitFor(() => {
      expect(screen.getByText(/network timeout/i)).toBeInTheDocument();
    });

    // All four preset buttons must still be in the DOM and not disabled
    for (const preset of ['minimal', 'balanced', 'extended', 'maximum']) {
      const btn = screen.getByTestId(`memory-window-option-${preset}`);
      expect(btn).toBeInTheDocument();
      expect(btn).not.toBeDisabled();
    }
  });
});
