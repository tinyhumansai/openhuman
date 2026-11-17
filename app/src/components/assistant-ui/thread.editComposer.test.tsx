import {
  type AppendMessage,
  AssistantRuntimeProvider,
  useExternalStoreRuntime,
} from '@assistant-ui/react';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { Thread } from './thread';

function Harness({
  onEdit = vi.fn(async () => {}),
}: {
  onEdit?: (message: AppendMessage) => Promise<void>;
}) {
  const runtime = useExternalStoreRuntime({
    messages: [{ id: 'user-1', role: 'user' as const, content: 'Original question' }],
    convertMessage: message => message,
    isRunning: false,
    onNew: vi.fn(async () => {}),
    onEdit,
  });
  return (
    <AssistantRuntimeProvider runtime={runtime}>
      <Thread />
    </AssistantRuntimeProvider>
  );
}

describe('native message edit composer', () => {
  it('disables sending an empty edit and cancels without changing the transcript', async () => {
    const onEdit = vi.fn(async () => {});
    render(<Harness onEdit={onEdit} />);
    fireEvent.click(screen.getByRole('button', { name: 'Edit' }));
    const input = screen.getByRole('textbox', { name: 'Edit your message' });
    expect(input).toHaveValue('Original question');
    fireEvent.change(input, { target: { value: '' } });
    expect(screen.getByRole('button', { name: 'Send' })).toBeDisabled();
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    await waitFor(() =>
      expect(screen.queryByRole('textbox', { name: 'Edit your message' })).toBeNull()
    );
    expect(screen.getByText('Original question')).toBeInTheDocument();
    expect(onEdit).not.toHaveBeenCalled();
  });

  it('sends the edit through the message composer rather than the thread composer', async () => {
    const onEdit = vi.fn(async () => {});
    render(<Harness onEdit={onEdit} />);
    fireEvent.click(screen.getByRole('button', { name: 'Edit' }));
    fireEvent.change(screen.getByRole('textbox', { name: 'Edit your message' }), {
      target: { value: 'Updated question' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Send' }));
    await waitFor(() =>
      expect(onEdit).toHaveBeenCalledWith(
        expect.objectContaining({ content: [{ type: 'text', text: 'Updated question' }] })
      )
    );
  });
});
