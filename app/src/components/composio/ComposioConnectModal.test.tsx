import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { authorize } from '../../lib/composio/composioApi';
import { type ComposioConnection } from '../../lib/composio/types';
import ComposioConnectModal, {
  isMissingRequiredFieldsError,
  isValidAtlassianSubdomain,
  sanitizeAuthError,
} from './ComposioConnectModal';
import { composioToolkitMeta } from './toolkitMeta';

vi.mock('../../lib/composio/composioApi', () => ({
  authorize: vi.fn(),
  deleteConnection: vi.fn(),
  getUserScopes: vi.fn(() => Promise.resolve({ read: true, write: true, admin: false })),
  listConnections: vi.fn(),
  setUserScopes: vi.fn(),
}));

vi.mock('../../utils/openUrl', () => ({ openUrl: vi.fn() }));

// Mock TriggerToggles because it does its own API calls
vi.mock('./TriggerToggles', () => ({ default: () => <div data-testid="trigger-toggles" /> }));

const mockToolkit = composioToolkitMeta('gmail');
const jiraToolkit = composioToolkitMeta('jira');

// ── Pure helper unit tests ────────────────────────────────────────────

describe('isValidAtlassianSubdomain', () => {
  it('accepts typical lowercase subdomain', () => {
    expect(isValidAtlassianSubdomain('acme')).toBe(true);
    expect(isValidAtlassianSubdomain('my-company')).toBe(true);
    expect(isValidAtlassianSubdomain('org123')).toBe(true);
  });

  it('accepts mixed-case subdomain (case-insensitive check)', () => {
    expect(isValidAtlassianSubdomain('MyCompany')).toBe(true);
  });

  it('accepts single-character subdomain', () => {
    expect(isValidAtlassianSubdomain('a')).toBe(true);
    expect(isValidAtlassianSubdomain('z')).toBe(true);
    expect(isValidAtlassianSubdomain('5')).toBe(true);
  });

  it('rejects full URLs', () => {
    expect(isValidAtlassianSubdomain('https://acme.atlassian.net')).toBe(false);
    expect(isValidAtlassianSubdomain('acme.atlassian.net')).toBe(false);
  });

  it('rejects leading/trailing hyphens', () => {
    expect(isValidAtlassianSubdomain('-acme')).toBe(false);
    expect(isValidAtlassianSubdomain('acme-')).toBe(false);
  });

  it('rejects empty string', () => {
    expect(isValidAtlassianSubdomain('')).toBe(false);
    expect(isValidAtlassianSubdomain('   ')).toBe(false);
  });

  it('rejects strings with spaces', () => {
    expect(isValidAtlassianSubdomain('my company')).toBe(false);
  });

  it('trims whitespace before validation', () => {
    expect(isValidAtlassianSubdomain('  acme  ')).toBe(true);
  });
});

describe('isMissingRequiredFieldsError', () => {
  it('matches the Composio error slug', () => {
    const err = new Error(
      'Authorization failed: [composio] authorize failed: Backend returned 400 Bad Request: Composio authorization failed: 400 {"error":{"message":"Missing required fields","code":612,"slug":"ConnectedAccount_MissingRequiredFields"}}'
    );
    expect(isMissingRequiredFieldsError(err)).toBe(true);
  });

  it('does NOT match on the numeric code alone — avoids false positives from port/resource numbers', () => {
    // The slug-only check prevents unrelated "612" occurrences (e.g. port numbers, IDs)
    // from being misidentified as the Composio missing-fields error.
    const err = new Error('error code 612 from server');
    expect(isMissingRequiredFieldsError(err)).toBe(false);
  });

  it('returns false for unrelated errors', () => {
    expect(isMissingRequiredFieldsError(new Error('Network timeout'))).toBe(false);
    expect(isMissingRequiredFieldsError(new Error('401 Unauthorized'))).toBe(false);
  });

  it('returns false for null / undefined', () => {
    expect(isMissingRequiredFieldsError(null)).toBe(false);
    expect(isMissingRequiredFieldsError(undefined)).toBe(false);
  });

  it('accepts non-Error objects with the slug in stringified form', () => {
    expect(isMissingRequiredFieldsError('ConnectedAccount_MissingRequiredFields')).toBe(true);
  });
});

describe('sanitizeAuthError', () => {
  it('returns a generic message for missing-required-fields errors', () => {
    const err = new Error(
      'Authorization failed: [composio] authorize failed: Backend returned 400 Bad Request for POST https://api.tinyhumans.ai/agent-integrations/composio/authorize: Composio authorization failed: 400 {"error":{"slug":"ConnectedAccount_MissingRequiredFields","code":612}}'
    );
    const result = sanitizeAuthError(err);
    expect(result).not.toContain('ConnectedAccount_MissingRequiredFields');
    expect(result).not.toContain('api.tinyhumans.ai');
    expect(result).not.toContain('612');
    expect(result).toContain('required field');
  });

  it('strips backend URLs from plain authorization errors', () => {
    const err = new Error(
      'Authorization failed: Backend returned 500 Internal Server Error for POST https://api.tinyhumans.ai/agent-integrations/composio/authorize: internal error'
    );
    const result = sanitizeAuthError(err);
    expect(result).not.toContain('api.tinyhumans.ai');
    expect(result).not.toContain('https://');
  });

  it('strips raw JSON payloads', () => {
    const err = new Error(
      'Authorization failed: something happened: {"error":{"code":500,"message":"internal"}}'
    );
    const result = sanitizeAuthError(err);
    expect(result).not.toContain('"code"');
    expect(result).not.toContain('"message"');
  });

  it('returns a safe fallback for null/undefined', () => {
    expect(sanitizeAuthError(null)).toBe('Something went wrong.');
    expect(sanitizeAuthError(undefined)).toBe('Something went wrong.');
  });

  it('handles non-Error thrown values', () => {
    const result = sanitizeAuthError('plain string error');
    expect(typeof result).toBe('string');
    expect(result.length).toBeGreaterThan(0);
  });
});

// ── Component render tests ────────────────────────────────────────────

describe('<ComposioConnectModal>', () => {
  it('hides raw connection ID and "id:" label in connected phase', () => {
    const connection: ComposioConnection = { id: 'ca_xyz', toolkit: 'gmail', status: 'ACTIVE' };

    render(
      <ComposioConnectModal toolkit={mockToolkit} connection={connection} onClose={() => {}} />
    );

    // Should be in 'connected' phase because connection.status is 'ACTIVE'
    expect(screen.getByText(/Gmail is connected/)).toBeInTheDocument();
    expect(screen.queryByText(/ca_xyz/)).not.toBeInTheDocument();
    expect(screen.queryByText(/id:/)).not.toBeInTheDocument();
  });

  it('renders accountEmail when provided', () => {
    const connection: ComposioConnection = {
      id: 'ca_xyz',
      toolkit: 'gmail',
      status: 'ACTIVE',
      accountEmail: 'foo@bar.com',
    };

    render(
      <ComposioConnectModal toolkit={mockToolkit} connection={connection} onClose={() => {}} />
    );

    expect(screen.getByText('(foo@bar.com)')).toBeInTheDocument();
  });

  it('renders workspace when accountEmail is missing', () => {
    const connection: ComposioConnection = {
      id: 'ca_xyz',
      toolkit: 'gmail',
      status: 'ACTIVE',
      workspace: 'Acme',
    };

    render(
      <ComposioConnectModal toolkit={mockToolkit} connection={connection} onClose={() => {}} />
    );

    expect(screen.getByText('(Acme)')).toBeInTheDocument();
  });

  it('renders username when email and workspace are missing', () => {
    const connection: ComposioConnection = {
      id: 'ca_xyz',
      toolkit: 'gmail',
      status: 'ACTIVE',
      username: 'oxox',
    };

    render(
      <ComposioConnectModal toolkit={mockToolkit} connection={connection} onClose={() => {}} />
    );

    expect(screen.getByText('(oxox)')).toBeInTheDocument();
  });

  it('prioritizes accountEmail over workspace and username', () => {
    const connection: ComposioConnection = {
      id: 'ca_xyz',
      toolkit: 'gmail',
      status: 'ACTIVE',
      accountEmail: 'foo@bar.com',
      workspace: 'Acme',
      username: 'oxox',
    };

    render(
      <ComposioConnectModal toolkit={mockToolkit} connection={connection} onClose={() => {}} />
    );

    expect(screen.getByText('(foo@bar.com)')).toBeInTheDocument();
    expect(screen.queryByText('(Acme)')).not.toBeInTheDocument();
    expect(screen.queryByText('(oxox)')).not.toBeInTheDocument();
  });
});

// ── Jira-specific flow tests ──────────────────────────────────────────

describe('<ComposioConnectModal> — Jira subdomain collection', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('shows the Atlassian subdomain input in the idle phase for Jira', () => {
    render(<ComposioConnectModal toolkit={jiraToolkit} onClose={() => {}} />);

    expect(screen.getByLabelText(/Atlassian subdomain/i)).toBeInTheDocument();
    expect(screen.getByPlaceholderText('your-subdomain')).toBeInTheDocument();
  });

  it('does NOT show the Atlassian subdomain input for non-Jira toolkits', () => {
    render(<ComposioConnectModal toolkit={mockToolkit} onClose={() => {}} />);

    expect(screen.queryByLabelText(/Atlassian subdomain/i)).not.toBeInTheDocument();
    expect(screen.queryByPlaceholderText('your-subdomain')).not.toBeInTheDocument();
  });

  it('shows a validation error when connect is clicked with an empty subdomain', async () => {
    render(<ComposioConnectModal toolkit={jiraToolkit} onClose={() => {}} />);

    const connectButton = screen.getByRole('button', { name: /Connect Jira/i });
    fireEvent.click(connectButton);

    await waitFor(() => {
      expect(screen.getByText(/Please enter your Atlassian subdomain/i)).toBeInTheDocument();
    });
  });

  it('shows a validation error when the subdomain looks like a full URL', async () => {
    render(<ComposioConnectModal toolkit={jiraToolkit} onClose={() => {}} />);

    const input = screen.getByPlaceholderText('your-subdomain');
    fireEvent.change(input, { target: { value: 'https://acme.atlassian.net' } });

    const connectButton = screen.getByRole('button', { name: /Connect Jira/i });
    fireEvent.click(connectButton);

    await waitFor(() => {
      expect(screen.getByText(/short subdomain only/i)).toBeInTheDocument();
    });
  });

  it('clears subdomain validation error when the user types', async () => {
    render(<ComposioConnectModal toolkit={jiraToolkit} onClose={() => {}} />);

    // Trigger validation error
    const connectButton = screen.getByRole('button', { name: /Connect Jira/i });
    fireEvent.click(connectButton);

    await waitFor(() => {
      expect(screen.getByText(/Please enter your Atlassian subdomain/i)).toBeInTheDocument();
    });

    // Type to clear the error
    const input = screen.getByPlaceholderText('your-subdomain');
    fireEvent.change(input, { target: { value: 'a' } });

    await waitFor(() => {
      expect(screen.queryByText(/Please enter your Atlassian subdomain/i)).not.toBeInTheDocument();
    });
  });
});

// ── needs-subdomain phase tests ───────────────────────────────────────

describe('<ComposioConnectModal> — needs-subdomain recovery phase', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('transitions to needs-subdomain phase for Jira when Composio returns the missing-required-fields error', async () => {
    // needs-subdomain phase is only shown for Atlassian toolkits (jira).
    vi.mocked(authorize).mockRejectedValueOnce(
      new Error(
        'Authorization failed: Backend returned 400: {"error":{"slug":"ConnectedAccount_MissingRequiredFields","code":612}}'
      )
    );

    render(<ComposioConnectModal toolkit={jiraToolkit} onClose={() => {}} />);

    const input = screen.getByPlaceholderText('your-subdomain');
    fireEvent.change(input, { target: { value: 'acme' } });
    fireEvent.click(screen.getByRole('button', { name: /Connect Jira/i }));

    await waitFor(() => {
      expect(screen.getByRole('button', { name: /Retry connection/i })).toBeInTheDocument();
      expect(screen.getByText(/To connect Jira/i)).toBeInTheDocument();
    });
  });

  it('routes non-Jira missing-required-fields errors to the error phase (not needs-subdomain)', async () => {
    // Gmail does not have an Atlassian subdomain — showing the Atlassian subdomain
    // form for it would be misleading and the retry would loop forever.
    vi.mocked(authorize).mockRejectedValueOnce(
      new Error(
        'Authorization failed: Backend returned 400: {"error":{"slug":"ConnectedAccount_MissingRequiredFields","code":612}}'
      )
    );

    render(<ComposioConnectModal toolkit={mockToolkit} onClose={() => {}} />);
    fireEvent.click(screen.getByRole('button', { name: /Connect Gmail/i }));

    await waitFor(() => {
      expect(screen.getByRole('button', { name: /Dismiss/i })).toBeInTheDocument();
      expect(screen.queryByRole('button', { name: /Retry connection/i })).not.toBeInTheDocument();
    });
  });

  it('does NOT show raw backend payload in the needs-subdomain phase', async () => {
    vi.mocked(authorize).mockRejectedValueOnce(
      new Error(
        'Authorization failed: Backend returned 400: {"error":{"slug":"ConnectedAccount_MissingRequiredFields","code":612,"message":"very sensitive backend payload"}}'
      )
    );

    render(<ComposioConnectModal toolkit={jiraToolkit} onClose={() => {}} />);

    const input = screen.getByPlaceholderText('your-subdomain');
    fireEvent.change(input, { target: { value: 'acme' } });
    fireEvent.click(screen.getByRole('button', { name: /Connect Jira/i }));

    await waitFor(() => {
      expect(screen.queryByText(/very sensitive backend payload/i)).not.toBeInTheDocument();
      expect(screen.queryByText(/ConnectedAccount_MissingRequiredFields/i)).not.toBeInTheDocument();
    });
  });

  it('clicking Cancel in needs-subdomain goes back to idle', async () => {
    vi.mocked(authorize).mockRejectedValueOnce(new Error('ConnectedAccount_MissingRequiredFields'));

    render(<ComposioConnectModal toolkit={jiraToolkit} onClose={() => {}} />);

    const input = screen.getByPlaceholderText('your-subdomain');
    fireEvent.change(input, { target: { value: 'acme' } });
    fireEvent.click(screen.getByRole('button', { name: /Connect Jira/i }));

    await waitFor(() => {
      expect(screen.getByRole('button', { name: /Retry connection/i })).toBeInTheDocument();
    });

    fireEvent.click(screen.getByRole('button', { name: /Cancel/i }));

    await waitFor(() => {
      expect(screen.getByRole('button', { name: /Connect Jira/i })).toBeInTheDocument();
    });
  });

  it('surfaces a sanitized (non-raw) error for unrelated authorization failures', async () => {
    vi.mocked(authorize).mockRejectedValueOnce(
      new Error(
        'Authorization failed: Backend returned 500 Internal Server Error for POST https://api.tinyhumans.ai/agent-integrations/composio/authorize: {"error":{"message":"internal server error payload","code":500}}'
      )
    );

    render(<ComposioConnectModal toolkit={mockToolkit} onClose={() => {}} />);

    fireEvent.click(screen.getByRole('button', { name: /Connect Gmail/i }));

    await waitFor(() => {
      // Should be in error phase, not needs-subdomain
      expect(screen.getByRole('button', { name: /Dismiss/i })).toBeInTheDocument();
      // Raw URL should not be shown
      expect(screen.queryByText(/api.tinyhumans.ai/i)).not.toBeInTheDocument();
      // Raw JSON payload should not be shown
      expect(screen.queryByText(/internal server error payload/i)).not.toBeInTheDocument();
    });
  });
});
