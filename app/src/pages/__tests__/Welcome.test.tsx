import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import Welcome from '../Welcome';
import { sendEmailMagicLink } from '../../services/api/authApi';
import { useDeepLinkAuthState } from '../../store/deepLinkAuthState';
import { isTauri } from '../../utils/tauriCommands';

vi.mock('../../components/RotatingTetrahedronCanvas', () => ({
  default: () => <div data-testid="welcome-logo" />,
}));

vi.mock('../../components/oauth/OAuthProviderButton', () => ({
  default: ({ provider }: { provider: { id: string } }) => <button>{provider.id}</button>,
}));

vi.mock('../../components/oauth/providerConfigs', () => ({
  oauthProviderConfigs: [{ id: 'google' }, { id: 'github' }, { id: 'twitter' }],
}));

vi.mock('../../services/api/authApi', () => ({
  sendEmailMagicLink: vi.fn(),
}));

vi.mock('../../store/deepLinkAuthState', () => ({
  useDeepLinkAuthState: vi.fn(),
}));

function createDeferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

describe('Welcome email login', () => {
  beforeEach(() => {
    vi.mocked(useDeepLinkAuthState).mockReturnValue({
      isProcessing: false,
      errorMessage: null,
    });
    vi.mocked(isTauri).mockReturnValue(false);
    vi.mocked(sendEmailMagicLink).mockReset();
  });

  it('uses the current origin for web email sign-in', async () => {
    vi.mocked(sendEmailMagicLink).mockResolvedValue(undefined);

    render(<Welcome />);

    fireEvent.change(screen.getByPlaceholderText('Enter your email'), {
      target: { value: 'user@example.com' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Continue with email' }));

    await waitFor(() => {
      expect(sendEmailMagicLink).toHaveBeenCalledWith('user@example.com', window.location.origin);
    });
  });

  it('uses the desktop deep-link URI for Tauri email sign-in', async () => {
    vi.mocked(isTauri).mockReturnValue(true);
    vi.mocked(sendEmailMagicLink).mockResolvedValue(undefined);

    render(<Welcome />);

    fireEvent.change(screen.getByPlaceholderText('Enter your email'), {
      target: { value: 'user@example.com' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Continue with email' }));

    await waitFor(() => {
      expect(sendEmailMagicLink).toHaveBeenCalledWith('user@example.com', 'openhuman://');
    });
  });

  it('shows a pending state while the email request is in flight', async () => {
    const deferred = createDeferred<void>();
    vi.mocked(sendEmailMagicLink).mockReturnValue(deferred.promise);

    render(<Welcome />);

    fireEvent.change(screen.getByPlaceholderText('Enter your email'), {
      target: { value: 'user@example.com' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Continue with email' }));

    expect(screen.getByRole('button', { name: /sending link/i })).toBeDisabled();
    expect(screen.getByText('Sending link...')).toBeInTheDocument();
    expect(screen.getByRole('status')).toHaveTextContent('Sending link...');

    deferred.resolve();
    await waitFor(() => {
      expect(screen.getByText('Check your email')).toBeInTheDocument();
    });
  });

  it('renders backend errors from the email request', async () => {
    vi.mocked(sendEmailMagicLink).mockRejectedValue(new Error('Email service is down'));

    render(<Welcome />);

    fireEvent.change(screen.getByPlaceholderText('Enter your email'), {
      target: { value: 'user@example.com' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Continue with email' }));

    await waitFor(() => {
      expect(screen.getByRole('alert')).toHaveTextContent('Email service is down');
    });
  });

  it('renders the success confirmation after a magic link is sent', async () => {
    vi.mocked(sendEmailMagicLink).mockResolvedValue(undefined);

    render(<Welcome />);

    fireEvent.change(screen.getByPlaceholderText('Enter your email'), {
      target: { value: 'user@example.com' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Continue with email' }));

    await waitFor(() => {
      expect(screen.getByText('Check your email')).toBeInTheDocument();
    });

    expect(screen.getByText('user@example.com')).toBeInTheDocument();
    expect(screen.getByRole('status')).toHaveTextContent('Check your email');
  });

  it('resets the success state when using a different email', async () => {
    vi.mocked(sendEmailMagicLink).mockResolvedValue(undefined);

    render(<Welcome />);

    fireEvent.change(screen.getByPlaceholderText('Enter your email'), {
      target: { value: 'user@example.com' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Continue with email' }));

    await waitFor(() => {
      expect(screen.getByText('Check your email')).toBeInTheDocument();
    });

    fireEvent.click(screen.getByRole('button', { name: 'Use a different email' }));

    expect(screen.queryByText('Check your email')).not.toBeInTheDocument();
    expect(screen.getByPlaceholderText('Enter your email')).toHaveValue('');
  });
});
