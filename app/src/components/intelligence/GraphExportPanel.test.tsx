import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import GraphExportPanel from './GraphExportPanel';

describe('<GraphExportPanel />', () => {
  it('renders the loading skeleton', () => {
    render(
      <GraphExportPanel
        count={null}
        format="json"
        onFormatChange={() => {}}
        onDownload={() => {}}
        loading
      />
    );
    expect(screen.getByTestId('graph-export-loading')).toBeInTheDocument();
  });

  it('renders the empty state when there is nothing to export', () => {
    render(
      <GraphExportPanel count={0} format="json" onFormatChange={() => {}} onDownload={() => {}} />
    );
    expect(screen.getByText('No knowledge graph to export yet.')).toBeInTheDocument();
  });

  it('renders an error with a working retry button', () => {
    const onRetry = vi.fn();
    render(
      <GraphExportPanel
        count={null}
        format="json"
        onFormatChange={() => {}}
        onDownload={() => {}}
        error="graph unavailable"
        onRetry={onRetry}
      />
    );
    expect(screen.getByRole('alert').textContent).toMatch(/graph unavailable/);
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    expect(onRetry).toHaveBeenCalledTimes(1);
  });

  it('shows the count, format toggle, and triggers download/format-change', () => {
    const onFormatChange = vi.fn();
    const onDownload = vi.fn();
    render(
      <GraphExportPanel
        count={42}
        format="json"
        onFormatChange={onFormatChange}
        onDownload={onDownload}
      />
    );
    expect(screen.getByText('42 facts ready to export.')).toBeInTheDocument();
    fireEvent.click(screen.getByLabelText('CSV'));
    expect(onFormatChange).toHaveBeenCalledWith('csv');
    fireEvent.click(screen.getByRole('button', { name: /Download JSON/ }));
    expect(onDownload).toHaveBeenCalledTimes(1);
  });
});
