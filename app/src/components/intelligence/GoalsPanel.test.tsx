import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { goalsApi } from '../../services/api/goalsApi';
import GoalsPanel from './GoalsPanel';

vi.mock('../../services/api/goalsApi', () => ({
  goalsApi: { list: vi.fn(), add: vi.fn(), edit: vi.fn(), remove: vi.fn(), reflect: vi.fn() },
}));

const api = vi.mocked(goalsApi);

describe('<GoalsPanel />', () => {
  beforeEach(() => vi.clearAllMocks());

  it('renders the empty state once loading resolves', async () => {
    api.list.mockResolvedValueOnce([]);
    render(<GoalsPanel />);
    expect(await screen.findByText(/No goals yet/)).toBeInTheDocument();
    expect(screen.getByText('Long-term Goals')).toBeInTheDocument();
  });

  it('renders loaded goals', async () => {
    api.list.mockResolvedValueOnce([
      { id: 'g1', text: 'Ship the desktop app' },
      { id: 'g2', text: 'Keep the Rust core authoritative' },
    ]);
    render(<GoalsPanel />);
    expect(await screen.findByText('Ship the desktop app')).toBeInTheDocument();
    expect(screen.getByText('Keep the Rust core authoritative')).toBeInTheDocument();
  });

  it('adds a goal from the input and renders the updated list', async () => {
    api.list.mockResolvedValueOnce([]);
    api.add.mockResolvedValueOnce([{ id: 'g1', text: 'New goal' }]);
    render(<GoalsPanel />);
    await screen.findByText(/No goals yet/);

    fireEvent.change(screen.getByPlaceholderText('Add a long-term goal…'), {
      target: { value: 'New goal' },
    });
    fireEvent.click(screen.getByRole('button', { name: /Add/ }));

    await waitFor(() => expect(api.add).toHaveBeenCalledWith('New goal'));
    expect(await screen.findByText('New goal')).toBeInTheDocument();
  });

  it('deletes a goal', async () => {
    api.list.mockResolvedValueOnce([{ id: 'g1', text: 'Doomed goal' }]);
    api.remove.mockResolvedValueOnce([]);
    render(<GoalsPanel />);
    await screen.findByText('Doomed goal');

    fireEvent.click(screen.getByRole('button', { name: 'Delete goal' }));
    await waitFor(() => expect(api.remove).toHaveBeenCalledWith('g1'));
  });

  it('runs reflect and shows the agent summary', async () => {
    api.list.mockResolvedValueOnce([]);
    api.reflect.mockResolvedValueOnce({
      ran: true,
      summary: 'Added 2 goals from context',
      items: [
        { id: 'g1', text: 'A' },
        { id: 'g2', text: 'B' },
      ],
    });
    render(<GoalsPanel />);
    await screen.findByText(/No goals yet/);

    fireEvent.click(screen.getByRole('button', { name: /Reflect/ }));
    await waitFor(() => expect(api.reflect).toHaveBeenCalled());
    expect(await screen.findByText('Added 2 goals from context')).toBeInTheDocument();
    expect(screen.getByText('A')).toBeInTheDocument();
  });

  it('surfaces an action error when add fails', async () => {
    api.list.mockResolvedValueOnce([]);
    api.add.mockRejectedValueOnce(new Error('boom'));
    render(<GoalsPanel />);
    await screen.findByText(/No goals yet/);

    fireEvent.change(screen.getByPlaceholderText('Add a long-term goal…'), {
      target: { value: 'x' },
    });
    fireEvent.click(screen.getByRole('button', { name: /Add/ }));
    expect(await screen.findByRole('alert')).toHaveTextContent('boom');
  });

  it('edits a goal inline and saves', async () => {
    api.list.mockResolvedValueOnce([{ id: 'g1', text: 'old text' }]);
    api.edit.mockResolvedValueOnce([{ id: 'g1', text: 'new text' }]);
    render(<GoalsPanel />);
    await screen.findByText('old text');

    fireEvent.click(screen.getByRole('button', { name: 'Edit goal' }));
    const input = screen.getByDisplayValue('old text');
    fireEvent.change(input, { target: { value: 'new text' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));

    await waitFor(() => expect(api.edit).toHaveBeenCalledWith('g1', 'new text'));
    expect(await screen.findByText('new text')).toBeInTheDocument();
  });

  it('cancels an inline edit without saving', async () => {
    api.list.mockResolvedValueOnce([{ id: 'g1', text: 'keep me' }]);
    render(<GoalsPanel />);
    await screen.findByText('keep me');

    fireEvent.click(screen.getByRole('button', { name: 'Edit goal' }));
    fireEvent.change(screen.getByDisplayValue('keep me'), { target: { value: 'discarded' } });
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));

    expect(api.edit).not.toHaveBeenCalled();
    expect(screen.getByText('keep me')).toBeInTheDocument();
  });

  it('surfaces an action error when edit fails', async () => {
    api.list.mockResolvedValueOnce([{ id: 'g1', text: 'x' }]);
    api.edit.mockRejectedValueOnce(new Error('edit blew up'));
    render(<GoalsPanel />);
    await screen.findByText('x');

    fireEvent.click(screen.getByRole('button', { name: 'Edit goal' }));
    fireEvent.change(screen.getByDisplayValue('x'), { target: { value: 'y' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('edit blew up');
  });

  it('surfaces an action error when delete fails', async () => {
    api.list.mockResolvedValueOnce([{ id: 'g1', text: 'x' }]);
    api.remove.mockRejectedValueOnce(new Error('delete failed'));
    render(<GoalsPanel />);
    await screen.findByText('x');

    fireEvent.click(screen.getByRole('button', { name: 'Delete goal' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('delete failed');
  });

  it('renders the load error branch when list rejects', async () => {
    api.list.mockRejectedValueOnce(new Error('cannot load goals'));
    render(<GoalsPanel />);
    expect(await screen.findByText('cannot load goals')).toBeInTheDocument();
  });

  it('shows the summary when reflect reports ran=false', async () => {
    api.list.mockResolvedValueOnce([]);
    api.reflect.mockResolvedValueOnce({
      ran: false,
      summary: 'enrichment failed: no model',
      items: [],
    });
    render(<GoalsPanel />);
    await screen.findByText(/No goals yet/);

    fireEvent.click(screen.getByRole('button', { name: /Reflect/ }));
    expect(await screen.findByText('enrichment failed: no model')).toBeInTheDocument();
  });

  it('surfaces an action error when reflect throws', async () => {
    api.list.mockResolvedValueOnce([]);
    api.reflect.mockRejectedValueOnce(new Error('reflect crashed'));
    render(<GoalsPanel />);
    await screen.findByText(/No goals yet/);

    fireEvent.click(screen.getByRole('button', { name: /Reflect/ }));
    expect(await screen.findByRole('alert')).toHaveTextContent('reflect crashed');
  });
});
