import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import * as composioApi from '../../lib/composio/composioApi';
import * as openUrlModule from '../../utils/openUrl';
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

  it('passes clearMemory only when the disconnect memory checkbox is selected', async () => {
    const connection: ComposioConnection = { id: 'ca_xyz', toolkit: 'gmail', status: 'ACTIVE' };
    vi.mocked(composioApi.deleteConnection).mockResolvedValue({
      deleted: true,
      memory_chunks_deleted: 1,
    });

    render(
      <ComposioConnectModal toolkit={mockToolkit} connection={connection} onClose={() => {}} />
    );

    fireEvent.click(screen.getByLabelText(/also delete memory/i));
    fireEvent.click(screen.getByRole('button', { name: /^Disconnect$/i }));

    await waitFor(() => {
      expect(composioApi.deleteConnection).toHaveBeenCalledWith('ca_xyz', { clearMemory: true });
    });
  });

  it('resets the clear-memory checkbox after a failed disconnect is dismissed', async () => {
    const connection: ComposioConnection = { id: 'ca_xyz', toolkit: 'gmail', status: 'ACTIVE' };
    vi.mocked(composioApi.deleteConnection).mockRejectedValueOnce(new Error('backend down'));

    render(
      <ComposioConnectModal toolkit={mockToolkit} connection={connection} onClose={() => {}} />
    );

    const checkbox = screen.getByLabelText(/also delete memory/i);
    fireEvent.click(checkbox);
    expect(checkbox).toBeChecked();

    fireEvent.click(screen.getByRole('button', { name: /^Disconnect$/i }));

    expect(await screen.findByText(/backend down/i)).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: /dismiss/i }));

    await waitFor(() => {
      expect(screen.getByLabelText(/also delete memory/i)).not.toBeChecked();
    });
  });

  it('shows an expired-auth recovery state with a reconnect CTA', () => {
    const connection: ComposioConnection = {
      id: 'ca_expired',
      toolkit: 'gmail',
      status: 'EXPIRED',
    };

    render(
      <ComposioConnectModal toolkit={mockToolkit} connection={connection} onClose={() => {}} />
    );

    expect(screen.getByText(/Gmail authorization expired/i)).toBeInTheDocument();
    expect(screen.getByText(/Reconnect to re-enable Gmail tools/i)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Reconnect Gmail/i })).toBeInTheDocument();
    expect(screen.queryByText(/ca_expired/)).not.toBeInTheDocument();
  });

  // ── Connect flow → openUrl(connectUrl) ───────────────────────────
  //
  // Verifies the end-to-end OAuth handoff plumbing for #1710:
  //   Connect click → authorize RPC → openUrl(connectUrl).
  //
  // The frontend doesn't care whether the URL is the backend's
  // `/agent-integrations/composio/authorize` redirect or Composio's
  // hosted `https://hosted.composio.dev/<token>` — both come back via
  // the same `connectUrl` field on `ComposioAuthorizeResponse`, so this
  // single assertion covers both modes. After this commit the
  // mechanically-wired direct-mode flow is: ops.rs `composio_authorize`
  // → factory routes to Direct → `direct_authorize` returns a Composio
  // hosted URL → frontend opens it in the system browser via this
  // path → `list_connections` polling detects the new ACTIVE row.
  describe('Connect flow (covers backend + direct mode #1710)', () => {
    beforeEach(() => {
      vi.mocked(composioApi.authorize).mockReset();
      vi.mocked(composioApi.listConnections).mockReset();
      vi.mocked(openUrlModule.openUrl).mockReset();
    });

    afterEach(() => {
      vi.useRealTimers();
    });

    it('opens the connectUrl from the authorize response in the system browser', async () => {
      // Direct mode emits an empty connectionId; backend mode emits a
      // populated one. The frontend treats both the same — only the
      // URL is the source of truth for opening the browser.
      vi.mocked(composioApi.authorize).mockResolvedValue({
        connectUrl: 'https://hosted.composio.dev/test-token',
        connectionId: '',
      });
      // Polling won't fire during this test — we don't advance timers
      // past the connect click, so listConnections only needs to be
      // mockable to avoid throwing.
      vi.mocked(composioApi.listConnections).mockResolvedValue({ connections: [] });

      render(<ComposioConnectModal toolkit={mockToolkit} onClose={() => {}} />);

      const connectBtn = screen.getByRole('button', { name: /Connect Gmail/ });
      fireEvent.click(connectBtn);

      await waitFor(() => {
        expect(composioApi.authorize).toHaveBeenCalledWith('gmail', undefined);
      });
      await waitFor(() => {
        expect(openUrlModule.openUrl).toHaveBeenCalledWith(
          'https://hosted.composio.dev/test-token'
        );
      });
    });

    it('shows the waiting state even when the OS browser opener rejects', async () => {
      vi.mocked(composioApi.authorize).mockResolvedValue({
        connectUrl: 'https://hosted.composio.dev/test-token',
        connectionId: '',
      });
      vi.mocked(composioApi.listConnections).mockResolvedValue({ connections: [] });
      vi.mocked(openUrlModule.openUrl).mockRejectedValueOnce(new Error('opener unavailable'));

      render(<ComposioConnectModal toolkit={mockToolkit} onClose={() => {}} />);

      fireEvent.click(screen.getByRole('button', { name: /Connect Gmail/ }));

      expect(await screen.findByRole('button', { name: /Reopen browser/i })).toBeInTheDocument();
      expect(screen.getByText(/Waiting for Gmail/i)).toBeInTheDocument();
      expect(screen.queryByText(/Something went wrong/i)).not.toBeInTheDocument();
    });
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
      expect(screen.getByText(/This field is required/i)).toBeInTheDocument();
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
      expect(screen.getByText(/This field is required/i)).toBeInTheDocument();
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

  it('surfaces Meta rate-limit guidance for Instagram authorize failures', async () => {
    const instagramToolkit = composioToolkitMeta('instagram');
    vi.mocked(authorize).mockRejectedValueOnce(
      new Error('Authorization failed: Backend returned 429 Too Many Requests')
    );

    render(<ComposioConnectModal toolkit={instagramToolkit} onClose={() => {}} />);
    fireEvent.click(screen.getByRole('button', { name: /Connect Instagram/i }));

    await waitFor(() => {
      expect(screen.getByText(/Business or Creator account/i)).toBeInTheDocument();
      expect(screen.getByText(/HTTP 429/i)).toBeInTheDocument();
      expect(screen.queryByText(/api.tinyhumans.ai/i)).not.toBeInTheDocument();
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

// ── Dynamics 365 org_name required-field flow (#2127) ──────────────────

describe('<ComposioConnectModal> — Dynamics 365 org_name collection (#2127)', () => {
  const dynamicsToolkit = composioToolkitMeta('dynamics365');

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders the Dynamics 365 Organization Name input in the idle phase', () => {
    render(<ComposioConnectModal toolkit={dynamicsToolkit} onClose={() => {}} />);

    expect(screen.getByLabelText(/Dynamics 365 Organization Name/i)).toBeInTheDocument();
    expect(screen.getByPlaceholderText('myorg')).toBeInTheDocument();
    // Suffix renders inside the input wrapper so users see the .crm.dynamics.com tail.
    expect(screen.getByText('.crm.dynamics.com')).toBeInTheDocument();
  });

  it('blocks submission when org name is empty and surfaces the generic required-field error', async () => {
    render(<ComposioConnectModal toolkit={dynamicsToolkit} onClose={() => {}} />);

    fireEvent.click(screen.getByRole('button', { name: /Connect Dynamics 365/i }));

    await waitFor(() => {
      expect(screen.getByText(/This field is required/i)).toBeInTheDocument();
    });
    expect(authorize).not.toHaveBeenCalled();
  });

  it('rejects a full URL with the subdomain-invalid message', async () => {
    render(<ComposioConnectModal toolkit={dynamicsToolkit} onClose={() => {}} />);

    const input = screen.getByPlaceholderText('myorg');
    fireEvent.change(input, { target: { value: 'https://myorg.crm.dynamics.com' } });
    fireEvent.click(screen.getByRole('button', { name: /Connect Dynamics 365/i }));

    await waitFor(() => {
      expect(screen.getByText(/short subdomain only/i)).toBeInTheDocument();
    });
    expect(authorize).not.toHaveBeenCalled();
  });

  it('forwards the trimmed org_name as extra_params on successful submit', async () => {
    vi.mocked(authorize).mockResolvedValue({
      connectUrl: 'https://hosted.composio.dev/dynamics-token',
      connectionId: 'ca_dyn_1',
    });
    vi.mocked(composioApi.listConnections).mockResolvedValue({ connections: [] });

    render(<ComposioConnectModal toolkit={dynamicsToolkit} onClose={() => {}} />);

    fireEvent.change(screen.getByPlaceholderText('myorg'), { target: { value: '  myorg  ' } });
    fireEvent.click(screen.getByRole('button', { name: /Connect Dynamics 365/i }));

    await waitFor(() => {
      expect(authorize).toHaveBeenCalledWith('dynamics365', { org_name: 'myorg' });
    });
  });

  it('transitions to the needs-fields recovery phase when Composio returns 612', async () => {
    vi.mocked(authorize).mockRejectedValueOnce(
      new Error(
        'Authorization failed: Backend returned 400: {"error":{"slug":"ConnectedAccount_MissingRequiredFields","code":612}}'
      )
    );

    render(<ComposioConnectModal toolkit={dynamicsToolkit} onClose={() => {}} />);

    fireEvent.change(screen.getByPlaceholderText('myorg'), { target: { value: 'myorg' } });
    fireEvent.click(screen.getByRole('button', { name: /Connect Dynamics 365/i }));

    await waitFor(() => {
      expect(screen.getByRole('button', { name: /Retry connection/i })).toBeInTheDocument();
      expect(screen.getByText(/we need a bit more information/i)).toBeInTheDocument();
    });
    expect(screen.queryByText(/ConnectedAccount_MissingRequiredFields/i)).not.toBeInTheDocument();
  });
});

// ── WhatsApp WABA id parity check — registry refactor must not regress (#2127) ─

describe('<ComposioConnectModal> — WhatsApp WABA id parity (#2127)', () => {
  const whatsappToolkit = composioToolkitMeta('whatsapp');

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('still renders the WABA id input and forwards waba_id as extra_params', async () => {
    vi.mocked(authorize).mockResolvedValue({
      connectUrl: 'https://hosted.composio.dev/wa-token',
      connectionId: 'ca_wa_1',
    });
    vi.mocked(composioApi.listConnections).mockResolvedValue({ connections: [] });

    render(<ComposioConnectModal toolkit={whatsappToolkit} onClose={() => {}} />);

    expect(screen.getByLabelText(/WhatsApp Business Account ID/i)).toBeInTheDocument();
    fireEvent.change(screen.getByPlaceholderText(/123456789012345/), {
      target: { value: '999000111222333' },
    });
    fireEvent.click(screen.getByRole('button', { name: /Connect WhatsApp/i }));

    await waitFor(() => {
      expect(authorize).toHaveBeenCalledWith('whatsapp', { waba_id: '999000111222333' });
    });
  });
});
