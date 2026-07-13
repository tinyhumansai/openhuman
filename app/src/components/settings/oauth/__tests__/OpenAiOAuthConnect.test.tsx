import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc } from '../../../../services/coreRpcClient';
import { renderWithProviders } from '../../../../test/test-utils';
import { openUrl } from '../../../../utils/openUrl';
import { isTauri } from '../../../../utils/tauriCommands/common';
import OpenAiOAuthConnect from '../OpenAiOAuthConnect';

vi.mock('../../../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));
vi.mock('../../../../utils/openUrl', () => ({ openUrl: vi.fn().mockResolvedValue(undefined) }));
vi.mock('../../../../utils/tauriCommands/common', () => ({ isTauri: vi.fn(() => true) }));

const TID = 'settings-openai-oauth';

describe('OpenAiOAuthConnect', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(isTauri).mockReturnValue(true);
    vi.mocked(openUrl).mockResolvedValue(undefined);
  });

  it('shows the connect button when status reports disconnected', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({ result: { connected: false } });

    renderWithProviders(<OpenAiOAuthConnect testIdPrefix={TID} />);

    expect(await screen.findByTestId(`${TID}-connect`)).toBeInTheDocument();
    expect(screen.queryByTestId(`${TID}-connected`)).not.toBeInTheDocument();
  });

  it('reflects an already-connected status and surfaces disconnect when allowed', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({ result: { connected: true } });
    const onConnectedChange = vi.fn();

    renderWithProviders(
      <OpenAiOAuthConnect
        testIdPrefix={TID}
        allowDisconnect
        onConnectedChange={onConnectedChange}
      />
    );

    expect(await screen.findByTestId(`${TID}-connected`)).toBeInTheDocument();
    expect(screen.getByText('Connected with ChatGPT')).toBeInTheDocument();
    expect(screen.getByTestId(`${TID}-disconnect`)).toBeInTheDocument();
    await waitFor(() => expect(onConnectedChange).toHaveBeenCalledWith(true));
  });

  it('does not repeat the connected callback when the parent callback identity changes', async () => {
    vi.mocked(callCoreRpc).mockResolvedValue({ result: { connected: true } });
    const first = vi.fn();
    const second = vi.fn();

    const { rerender } = renderWithProviders(
      <OpenAiOAuthConnect testIdPrefix={TID} onConnectedChange={first} />
    );
    await waitFor(() => expect(first).toHaveBeenCalledTimes(1));

    rerender(<OpenAiOAuthConnect testIdPrefix={TID} onConnectedChange={second} />);

    await waitFor(() => expect(callCoreRpc).toHaveBeenCalledTimes(1));
    expect(second).not.toHaveBeenCalled();
  });

  it('hides disconnect when not connected even if allowDisconnect is set', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({ result: { connected: false } });

    renderWithProviders(<OpenAiOAuthConnect testIdPrefix={TID} allowDisconnect />);

    expect(await screen.findByTestId(`${TID}-connect`)).toBeInTheDocument();
    expect(screen.queryByTestId(`${TID}-disconnect`)).not.toBeInTheDocument();
  });

  it('runs the start → paste callback → complete flow and reports connection', async () => {
    vi.mocked(callCoreRpc)
      .mockResolvedValueOnce({ result: { connected: false } }) // status
      .mockResolvedValueOnce({ result: { authUrl: 'https://auth.openai.com/oauth?x=1' } }) // start
      .mockResolvedValueOnce({ result: {} }); // complete
    const onConnectedChange = vi.fn();

    renderWithProviders(
      <OpenAiOAuthConnect testIdPrefix={TID} onConnectedChange={onConnectedChange} />
    );

    fireEvent.click(await screen.findByTestId(`${TID}-connect`));

    await waitFor(() => expect(openUrl).toHaveBeenCalledWith('https://auth.openai.com/oauth?x=1'));

    const input = await screen.findByTestId(`${TID}-callback-input`);
    fireEvent.change(input, {
      target: { value: 'http://127.0.0.1:1455/auth/callback?code=abc&state=xyz' },
    });
    fireEvent.click(screen.getByTestId(`${TID}-complete`));

    expect(await screen.findByTestId(`${TID}-connected`)).toBeInTheDocument();
    await waitFor(() => expect(onConnectedChange).toHaveBeenCalledWith(true));
    expect(callCoreRpc).toHaveBeenLastCalledWith({
      method: 'openhuman.inference_openai_oauth_complete',
      params: { callback_url: 'http://127.0.0.1:1455/auth/callback?code=abc&state=xyz' },
    });
  });

  it('disconnects and returns to the connect button', async () => {
    vi.mocked(callCoreRpc)
      .mockResolvedValueOnce({ result: { connected: true } }) // status
      .mockResolvedValueOnce({ result: { removed: true } }); // disconnect
    const onConnectedChange = vi.fn();

    renderWithProviders(
      <OpenAiOAuthConnect
        testIdPrefix={TID}
        allowDisconnect
        onConnectedChange={onConnectedChange}
      />
    );

    fireEvent.click(await screen.findByTestId(`${TID}-disconnect`));

    expect(await screen.findByTestId(`${TID}-connect`)).toBeInTheDocument();
    await waitFor(() => expect(onConnectedChange).toHaveBeenLastCalledWith(false));
    expect(callCoreRpc).toHaveBeenLastCalledWith({
      method: 'openhuman.inference_openai_oauth_disconnect',
      params: {},
    });
  });

  it('blocks sign-in outside the desktop app', async () => {
    vi.mocked(isTauri).mockReturnValue(false);

    renderWithProviders(<OpenAiOAuthConnect testIdPrefix={TID} />);

    fireEvent.click(await screen.findByTestId(`${TID}-connect`));

    expect(await screen.findByTestId(`${TID}-error`)).toHaveTextContent(
      'ChatGPT sign-in is only available in the desktop app.'
    );
    // No RPC should have been attempted (status probe also short-circuits off-desktop).
    expect(callCoreRpc).not.toHaveBeenCalled();
  });

  it('requires a callback URL before completing', async () => {
    vi.mocked(callCoreRpc)
      .mockResolvedValueOnce({ result: { connected: false } }) // status
      .mockResolvedValueOnce({ result: { authUrl: 'https://auth.openai.com/oauth?x=1' } }); // start

    renderWithProviders(<OpenAiOAuthConnect testIdPrefix={TID} />);

    fireEvent.click(await screen.findByTestId(`${TID}-connect`));
    fireEvent.click(await screen.findByTestId(`${TID}-complete`));

    expect(await screen.findByTestId(`${TID}-error`)).toHaveTextContent(
      'Paste the redirect URL from your browser after signing in.'
    );
  });

  it('surfaces a start error when the RPC fails', async () => {
    vi.mocked(callCoreRpc)
      .mockResolvedValueOnce({ result: { connected: false } }) // status
      .mockRejectedValueOnce(new Error('boom')); // start

    renderWithProviders(<OpenAiOAuthConnect testIdPrefix={TID} />);

    fireEvent.click(await screen.findByTestId(`${TID}-connect`));

    expect(await screen.findByTestId(`${TID}-error`)).toHaveTextContent(
      'Could not start ChatGPT sign-in.'
    );
  });

  it('does not show the callback form when opening the browser fails', async () => {
    vi.mocked(callCoreRpc)
      .mockResolvedValueOnce({ result: { connected: false } })
      .mockResolvedValueOnce({ result: { authUrl: 'https://auth.openai.com/oauth?x=1' } });
    vi.mocked(openUrl).mockRejectedValueOnce(new Error('shell open failed'));

    renderWithProviders(<OpenAiOAuthConnect testIdPrefix={TID} />);
    fireEvent.click(await screen.findByTestId(`${TID}-connect`));

    expect(await screen.findByTestId(`${TID}-error`)).toHaveTextContent(
      'Could not start ChatGPT sign-in.'
    );
    expect(screen.queryByTestId(`${TID}-callback-input`)).not.toBeInTheDocument();
  });

  it('surfaces a completion error when the RPC fails', async () => {
    vi.mocked(callCoreRpc)
      .mockResolvedValueOnce({ result: { connected: false } }) // status
      .mockResolvedValueOnce({ result: { authUrl: 'https://auth.openai.com/oauth?x=1' } }) // start
      .mockRejectedValueOnce(new Error('boom')); // complete

    renderWithProviders(<OpenAiOAuthConnect testIdPrefix={TID} />);

    fireEvent.click(await screen.findByTestId(`${TID}-connect`));
    const input = await screen.findByTestId(`${TID}-callback-input`);
    fireEvent.change(input, { target: { value: 'http://127.0.0.1:1455/auth/callback?code=a' } });
    fireEvent.click(screen.getByTestId(`${TID}-complete`));

    expect(await screen.findByTestId(`${TID}-error`)).toHaveTextContent(
      'ChatGPT sign-in did not complete.'
    );
  });

  it('surfaces a disconnect error when the RPC fails', async () => {
    vi.mocked(callCoreRpc)
      .mockResolvedValueOnce({ result: { connected: true } }) // status
      .mockRejectedValueOnce(new Error('boom')); // disconnect

    renderWithProviders(<OpenAiOAuthConnect testIdPrefix={TID} allowDisconnect />);

    fireEvent.click(await screen.findByTestId(`${TID}-disconnect`));

    expect(await screen.findByTestId(`${TID}-error`)).toHaveTextContent(
      'Could not disconnect ChatGPT.'
    );
  });

  it('stays on the connect button when the status probe fails', async () => {
    vi.mocked(callCoreRpc).mockRejectedValueOnce(new Error('probe failed')); // status

    renderWithProviders(<OpenAiOAuthConnect testIdPrefix={TID} />);

    expect(await screen.findByTestId(`${TID}-connect`)).toBeInTheDocument();
    expect(screen.queryByTestId(`${TID}-connected`)).not.toBeInTheDocument();
  });
});
