import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { GraphRelation } from '../../utils/tauriCommands/memory';
import GraphExportTab from './GraphExportTab';

const mockLoad = vi.fn();

vi.mock('../../services/api/graphExportApi', () => ({
  loadGraphRelations: (...args: unknown[]) => mockLoad(...args),
}));

function rel(subject: string, object: string): GraphRelation {
  return {
    namespace: 'work',
    subject,
    predicate: 'knows',
    object,
    attrs: {},
    updatedAt: 0,
    evidenceCount: 1,
    orderIndex: null,
    documentIds: [],
    chunkIds: [],
  };
}

describe('<GraphExportTab />', () => {
  beforeEach(() => {
    mockLoad.mockReset();
    mockLoad.mockResolvedValue([rel('A', 'B'), rel('B', 'C')]);
    // jsdom lacks URL.createObjectURL / revokeObjectURL — stub them.
    (URL as unknown as { createObjectURL: () => string }).createObjectURL = vi.fn(
      () => 'blob:test'
    );
    (URL as unknown as { revokeObjectURL: () => void }).revokeObjectURL = vi.fn();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('loads on mount and shows the export-ready count', async () => {
    render(<GraphExportTab />);
    expect(mockLoad).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(screen.getByText('2 facts ready to export.')).toBeInTheDocument());
  });

  it('triggers a Blob download when Download is clicked', async () => {
    const clickSpy = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {});
    render(<GraphExportTab />);
    await waitFor(() => screen.getByRole('button', { name: /Download/ }));
    fireEvent.click(screen.getByRole('button', { name: /Download/ }));
    expect(URL.createObjectURL).toHaveBeenCalledTimes(1);
    expect(clickSpy).toHaveBeenCalledTimes(1);
  });

  it('surfaces an error when the load fails', async () => {
    mockLoad.mockReset();
    mockLoad.mockRejectedValueOnce(new Error('graph unavailable'));
    render(<GraphExportTab />);
    await waitFor(() => expect(screen.getByRole('alert').textContent).toMatch(/graph unavailable/));
  });
});
