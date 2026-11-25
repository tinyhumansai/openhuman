import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import KeyringOperationApprovalOverlay from './KeyringOperationApprovalOverlay';

const mocks = vi.hoisted(() => ({
  dismiss: vi.fn(),
  respond: vi.fn(),
  requests: [
    {
      request_id: 'request-1',
      caller_agent: 'OpenHuman',
      origin: 'local_operator',
      purpose: 'Read credential profile',
      access: 'read' as const,
      name: 'auth:openai:default',
      expires_at_ms: Date.now() + 60_000,
    },
  ],
}));

vi.mock('../../hooks/useKeyringOperationApprovals', () => ({
  useKeyringOperationApprovals: () => ({ requests: mocks.requests, dismiss: mocks.dismiss }),
}));
vi.mock('../../services/keyringApi', () => ({
  respondToKeyringOperation: (...args: unknown[]) => mocks.respond(...args),
}));
vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

describe('KeyringOperationApprovalOverlay', () => {
  beforeEach(() => {
    mocks.dismiss.mockReset();
    mocks.respond.mockReset().mockResolvedValue(undefined);
  });

  it('shows the exact operation metadata without any secret value', () => {
    render(<KeyringOperationApprovalOverlay />);

    expect(screen.getByRole('dialog', { name: 'keyring.operation.title' })).toBeInTheDocument();
    expect(screen.getByText('OpenHuman')).toBeInTheDocument();
    expect(screen.getByText('local_operator')).toBeInTheDocument();
    expect(screen.getByText('Read credential profile')).toBeInTheDocument();
    expect(screen.getByText('auth:openai:default')).toBeInTheDocument();
    expect(screen.getByText('keyring.operation.access.read')).toBeInTheDocument();
  });

  it('sends an approval for the displayed request and dismisses it after success', async () => {
    render(<KeyringOperationApprovalOverlay />);
    fireEvent.click(screen.getByRole('button', { name: 'keyring.operation.allow' }));

    await waitFor(() => expect(mocks.respond).toHaveBeenCalledWith('request-1', true));
    expect(mocks.dismiss).toHaveBeenCalledWith('request-1');
  });

  it('sends a denial when the user denies the request', async () => {
    render(<KeyringOperationApprovalOverlay />);
    fireEvent.click(screen.getByRole('button', { name: 'keyring.operation.deny' }));

    await waitFor(() => expect(mocks.respond).toHaveBeenCalledWith('request-1', false));
    expect(mocks.dismiss).toHaveBeenCalledWith('request-1');
  });

  it('keeps the request visible and reports a generic error when the decision fails', async () => {
    mocks.respond.mockRejectedValueOnce(new Error('sensitive backend detail'));
    render(<KeyringOperationApprovalOverlay />);
    fireEvent.click(screen.getByRole('button', { name: 'keyring.operation.allow' }));

    expect(await screen.findByText('keyring.operation.error')).toBeInTheDocument();
    expect(screen.queryByText('sensitive backend detail')).not.toBeInTheDocument();
    expect(mocks.dismiss).not.toHaveBeenCalled();
  });
});
