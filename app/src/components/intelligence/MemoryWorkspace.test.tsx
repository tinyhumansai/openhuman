import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { memoryTreeGraphExport } from '../../utils/tauriCommands';
import { MemoryWorkspace } from './MemoryWorkspace';

// Stub the i18n hook and the heavy child panels so the test renders just the
// MemoryWorkspace shell (graph fetch effect + the refresh control + poll).
vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));
vi.mock('./MemoryGraph', () => ({ MemoryGraph: () => null }));
vi.mock('./MemorySourcesRegistry', () => ({ MemorySourcesRegistry: () => null }));
vi.mock('./MemoryTreeStatusPanel', () => ({ MemoryTreeStatusPanel: () => null }));
vi.mock('./ObsidianVaultSection', () => ({ ObsidianVaultSection: () => null }));
vi.mock('./WhatsAppMemorySection', () => ({ WhatsAppMemorySection: () => null }));
vi.mock('../../utils/tauriCommands', () => ({
  memoryTreeGraphExport: vi.fn().mockResolvedValue({ nodes: [], edges: [] }),
  memoryTreeFlushNow: vi.fn().mockResolvedValue(undefined),
  memoryTreeResetTree: vi.fn().mockResolvedValue(undefined),
  memoryTreeWipeAll: vi.fn().mockResolvedValue(undefined),
}));

const graphExportMock = vi.mocked(memoryTreeGraphExport);

describe('<MemoryWorkspace />', () => {
  beforeEach(() => {
    graphExportMock.mockClear();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it('re-exports the graph when the refresh button is clicked', async () => {
    render(<MemoryWorkspace />);
    // Mount runs the initial graph load.
    await waitFor(() => expect(graphExportMock).toHaveBeenCalledTimes(1));

    fireEvent.click(screen.getByTestId('memory-graph-refresh'));

    // Bumping graphVersion re-runs the load effect.
    await waitFor(() => expect(graphExportMock).toHaveBeenCalledTimes(2));
  });

  it('polls the graph on a 30s tick, and skips the tick while the tab is hidden', async () => {
    vi.useFakeTimers();
    render(<MemoryWorkspace />);
    // Flush the mount-time async load under fake timers.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    const afterMount = graphExportMock.mock.calls.length;

    // Visible tab → the 30s tick re-pulls the graph.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(30_000);
    });
    expect(graphExportMock.mock.calls.length).toBeGreaterThan(afterMount);

    // Backgrounded tab → the tick is skipped (no extra RPC churn).
    const hiddenSpy = vi.spyOn(document, 'hidden', 'get').mockReturnValue(true);
    const afterVisible = graphExportMock.mock.calls.length;
    await act(async () => {
      await vi.advanceTimersByTimeAsync(30_000);
    });
    expect(graphExportMock.mock.calls.length).toBe(afterVisible);

    hiddenSpy.mockRestore();
  });
});
