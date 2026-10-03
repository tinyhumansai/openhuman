import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { MemoryContext } from '../../services/api/memoryApi';
import { renderWithProviders } from '../../test/test-utils';
import MemoryContextTab from './MemoryContextTab';

const hoisted = vi.hoisted(() => ({ get: vi.fn(), refresh: vi.fn(), set: vi.fn() }));

vi.mock('../../services/api/memoryApi', async importOriginal => ({
  ...(await importOriginal<typeof import('../../services/api/memoryApi')>()),
  memoryContextGet: (...a: unknown[]) => hoisted.get(...a),
  memoryContextRefresh: (...a: unknown[]) => hoisted.refresh(...a),
  memoryContextSet: (...a: unknown[]) => hoisted.set(...a),
}));

const CTX: MemoryContext = {
  markdown: '# About you\n\n- Works on **OpenHuman**',
  tokens: 321,
  generated_at: '2026-10-01T09:00:00Z',
  interval_mins: 360,
  budget_tokens: 2000,
  enabled: true,
};

beforeEach(() => {
  hoisted.get.mockReset().mockResolvedValue(CTX);
  hoisted.refresh.mockReset();
  hoisted.set.mockReset();
});

describe('MemoryContextTab', () => {
  it('renders the brief as markdown with its token count', async () => {
    renderWithProviders(<MemoryContextTab />);
    const md = await screen.findByTestId('memory-context-markdown');
    expect(md.querySelector('h1')).toHaveTextContent('About you');
    expect(md.querySelector('strong')).toHaveTextContent('OpenHuman');
    expect(screen.getByTestId('memory-context-brief')).toHaveTextContent('321 tokens');
    expect(screen.getByLabelText('Regenerate every')).toHaveValue(360);
    expect(screen.getByLabelText('Size limit')).toHaveValue(2000);
  });

  it('says when the brief has never been generated', async () => {
    hoisted.get.mockResolvedValue({ ...CTX, markdown: '', generated_at: null, tokens: 0 });
    renderWithProviders(<MemoryContextTab />);
    expect(await screen.findByTestId('memory-context-brief')).toHaveTextContent(
      'Not generated yet'
    );
    expect(screen.getByTestId('memory-context-markdown')).toHaveTextContent('The brief is empty.');
  });

  it('regenerates the brief', async () => {
    hoisted.refresh.mockResolvedValue({ ...CTX, markdown: 'Fresh brief', tokens: 12 });
    renderWithProviders(<MemoryContextTab />);
    fireEvent.click(await screen.findByTestId('memory-context-regenerate'));
    await waitFor(() => expect(hoisted.refresh).toHaveBeenCalled());
    expect(await screen.findByText('Fresh brief')).toBeInTheDocument();
  });

  it('saves the schedule and budget', async () => {
    hoisted.set.mockImplementation(async (u: Partial<MemoryContext>) => ({ ...CTX, ...u }));
    renderWithProviders(<MemoryContextTab />);
    const interval = await screen.findByLabelText('Regenerate every');
    fireEvent.change(interval, { target: { value: '60' } });
    fireEvent.blur(interval);
    await waitFor(() => expect(hoisted.set).toHaveBeenCalledWith({ interval_mins: 60 }));

    fireEvent.click(screen.getByTestId('memory-context-enabled'));
    await waitFor(() => expect(hoisted.set).toHaveBeenCalledWith({ enabled: false }));
  });

  it('shows a regenerate failure', async () => {
    hoisted.refresh.mockRejectedValue(new Error('ENGINE: answer route failed'));
    renderWithProviders(<MemoryContextTab />);
    fireEvent.click(await screen.findByTestId('memory-context-regenerate'));
    expect(await screen.findByTestId('memory-context-error')).toHaveTextContent(
      'answer route failed'
    );
  });

  it('shows a load error', async () => {
    hoisted.get.mockRejectedValue(new Error('MEMORY_OFF'));
    renderWithProviders(<MemoryContextTab />);
    expect(await screen.findByTestId('memory-context-error')).toHaveTextContent('MEMORY_OFF');
  });
});
