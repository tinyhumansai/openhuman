import { screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import type { ToastNotification } from '../../types/intelligence';
import MemorySection from './MemorySection';

const memoryWorkspaceSpy =
  vi.fn<(props: { onToast?: (toast: Omit<ToastNotification, 'id'>) => void }) => void>();

vi.mock('./MemoryWorkspace', () => ({
  MemoryWorkspace: (props: { onToast?: (toast: Omit<ToastNotification, 'id'>) => void }) => {
    memoryWorkspaceSpy(props);
    return <div data-testid="memory-workspace" />;
  },
}));

describe('<MemorySection />', () => {
  it('renders the core MemoryWorkspace directly with no analysis sub-pill bar', () => {
    const onToast = vi.fn();
    renderWithProviders(<MemorySection onToast={onToast} />);

    // Core view is shown.
    expect(screen.getByTestId('memory-workspace')).toBeInTheDocument();

    // No sub-pill/tab bar (the analysis pills are gone).
    for (const label of [
      'Diagram',
      'Centrality',
      'Cohesion',
      'Associations',
      'Freshness',
      'Timeline',
      'Paths',
      'Namespaces',
    ]) {
      expect(screen.queryByRole('button', { name: label })).not.toBeInTheDocument();
    }
  });

  it('forwards onToast to MemoryWorkspace', () => {
    const onToast = vi.fn();
    renderWithProviders(<MemorySection onToast={onToast} />);

    expect(memoryWorkspaceSpy).toHaveBeenCalledWith(expect.objectContaining({ onToast }));
  });
});
