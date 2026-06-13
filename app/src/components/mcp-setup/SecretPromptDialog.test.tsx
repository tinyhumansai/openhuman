import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { SecretPromptDialog } from './SecretPromptDialog';

const callCoreRpc = vi.fn();
vi.mock('../../services/coreRpcClient', () => ({
  callCoreRpc: (...args: unknown[]) => callCoreRpc(...args),
}));

function dispatchRequest(detail: { refId: string; keyName: string; prompt: string }) {
  window.dispatchEvent(new CustomEvent('openhuman:mcp-setup-secret-requested', { detail }));
}

describe('SecretPromptDialog', () => {
  beforeEach(() => {
    callCoreRpc.mockReset();
    callCoreRpc.mockResolvedValue(undefined);
  });

  afterEach(() => {
    // The component cleans up its own listener on unmount via useEffect;
    // we don't need to remove the event listener manually here.
  });

  it('is hidden until an event is dispatched', () => {
    render(<SecretPromptDialog />);
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('renders the prompt + key name when an event arrives', async () => {
    render(<SecretPromptDialog />);
    dispatchRequest({
      refId: 'secret://abc123',
      keyName: 'NOTION_API_KEY',
      prompt: 'Paste your Notion integration token.',
    });
    await screen.findByRole('dialog');
    expect(screen.getByText('NOTION_API_KEY')).toBeTruthy();
    expect(screen.getByText('Paste your Notion integration token.')).toBeTruthy();
  });

  it('submits the value via mcp_setup_submit_secret and dismisses', async () => {
    render(<SecretPromptDialog />);
    dispatchRequest({ refId: 'secret://abc123', keyName: 'TOKEN', prompt: '' });
    await screen.findByRole('dialog');

    const input = screen.getByLabelText(/^Value$/i);
    fireEvent.change(input, { target: { value: 'super-secret-value' } });

    const submit = screen.getByText(/^Submit$/);
    fireEvent.click(submit);

    await waitFor(() => {
      expect(callCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.mcp_setup_submit_secret',
        params: { ref_id: 'secret://abc123', value: 'super-secret-value' },
      });
    });
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  });

  it('renders input as type=password by default and toggles on Show/Hide', async () => {
    render(<SecretPromptDialog />);
    dispatchRequest({ refId: 'secret://abc', keyName: 'K', prompt: '' });
    await screen.findByRole('dialog');

    const input = screen.getByLabelText(/^Value$/i) as HTMLInputElement;
    expect(input.type).toBe('password');

    fireEvent.click(screen.getByText(/^Show$/));
    expect((screen.getByLabelText(/^Value$/i) as HTMLInputElement).type).toBe('text');

    fireEvent.click(screen.getByText(/^Hide$/));
    expect((screen.getByLabelText(/^Value$/i) as HTMLInputElement).type).toBe('password');
  });

  it('cancel does not call mcp_setup_submit_secret', async () => {
    render(<SecretPromptDialog />);
    dispatchRequest({ refId: 'secret://abc', keyName: 'K', prompt: '' });
    await screen.findByRole('dialog');

    fireEvent.click(screen.getByText(/^Cancel$/));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(callCoreRpc).not.toHaveBeenCalled();
  });

  it('submit button disabled when value is empty', async () => {
    render(<SecretPromptDialog />);
    dispatchRequest({ refId: 'secret://abc', keyName: 'K', prompt: '' });
    await screen.findByRole('dialog');

    const submit = screen.getByText(/^Submit$/) as HTMLButtonElement;
    expect(submit.disabled).toBe(true);

    const input = screen.getByLabelText(/^Value$/i);
    fireEvent.change(input, { target: { value: 'x' } });
    expect(submit.disabled).toBe(false);
  });

  it('surfaces submit errors without dismissing', async () => {
    callCoreRpc.mockRejectedValueOnce(new Error('boom'));
    render(<SecretPromptDialog />);
    dispatchRequest({ refId: 'secret://abc', keyName: 'K', prompt: '' });
    await screen.findByRole('dialog');

    const input = screen.getByLabelText(/^Value$/i);
    fireEvent.change(input, { target: { value: 'v' } });
    fireEvent.click(screen.getByText(/^Submit$/));

    await waitFor(() => expect(screen.getByText(/boom/)).toBeTruthy());
    // Dialog still open
    expect(screen.queryByRole('dialog')).not.toBeNull();
  });
});
