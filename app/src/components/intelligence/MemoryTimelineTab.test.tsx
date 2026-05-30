import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { computeTimeline } from '../../lib/memory/memoryTimeline';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import MemoryTimelineTab from './MemoryTimelineTab';

const mockLoadTimeline = vi.fn();
const mockLoadNamespaces = vi.fn();

vi.mock('../../services/api/memoryTimelineApi', () => ({
  loadTimeline: (...args: unknown[]) => mockLoadTimeline(...args),
  loadNamespaces: (...args: unknown[]) => mockLoadNamespaces(...args),
}));

const NOW = 1_700_000_000;

function rel(updatedAt: number): GraphRelation {
  return {
    namespace: 'n',
    subject: 'You',
    predicate: 'p',
    object: 'x',
    attrs: {},
    updatedAt,
    evidenceCount: 1,
    orderIndex: null,
    documentIds: [],
    chunkIds: [],
  };
}

const report = computeTimeline([rel(Math.floor(Date.UTC(2023, 0, 10) / 1000))], NOW);

describe('<MemoryTimelineTab />', () => {
  beforeEach(() => {
    mockLoadTimeline.mockReset();
    mockLoadNamespaces.mockReset();
    mockLoadTimeline.mockResolvedValue(report);
    mockLoadNamespaces.mockResolvedValue([]);
  });

  it('loads the timeline on mount and renders it', async () => {
    render(<MemoryTimelineTab />);
    expect(mockLoadTimeline).toHaveBeenCalledTimes(1);
    expect(mockLoadTimeline.mock.calls[0][1]).toBeUndefined(); // (nowSeconds, undefined-namespace)
    await waitFor(() => expect(screen.getByText('Facts learned per month')).toBeInTheDocument());
  });

  it('shows the namespace selector and re-queries on change', async () => {
    mockLoadNamespaces.mockResolvedValueOnce(['work', 'personal']);
    render(<MemoryTimelineTab />);
    await waitFor(() => screen.getByRole('combobox'));
    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'work' } });
    await waitFor(() => expect(mockLoadTimeline).toHaveBeenCalledTimes(2));
    expect(mockLoadTimeline.mock.calls[1][1]).toBe('work');
  });

  it('surfaces an error when the load fails', async () => {
    mockLoadTimeline.mockReset();
    mockLoadTimeline.mockRejectedValueOnce(new Error('graph unavailable'));
    render(<MemoryTimelineTab />);
    await waitFor(() => expect(screen.getByRole('alert').textContent).toMatch(/graph unavailable/));
  });
});
