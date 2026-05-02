import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import TriggerToggles, { activeTriggerSignature, triggerSignature } from './TriggerToggles';

const mockListAvailable = vi.fn();
const mockListTriggers = vi.fn();
const mockEnable = vi.fn();
const mockDisable = vi.fn();

vi.mock('../../lib/composio/composioApi', () => ({
  listAvailableTriggers: (toolkit: string, conn?: string) => mockListAvailable(toolkit, conn),
  listTriggers: (toolkit?: string) => mockListTriggers(toolkit),
  enableTrigger: (conn: string, slug: string, cfg?: Record<string, unknown>) =>
    mockEnable(conn, slug, cfg),
  disableTrigger: (id: string) => mockDisable(id),
}));

beforeEach(() => {
  mockListAvailable.mockReset();
  mockListTriggers.mockReset();
  mockEnable.mockReset();
  mockDisable.mockReset();
});

describe('triggerSignature / activeTriggerSignature', () => {
  it('keys static triggers by uppercase slug', () => {
    expect(triggerSignature('gmail_new', 'static')).toBe('GMAIL_NEW');
  });

  it('keys github triggers by slug + lowercase owner/repo', () => {
    expect(
      triggerSignature('GITHUB_PUSH_EVENT', 'github_repo', { owner: 'Acme', repo: 'API' })
    ).toBe('GITHUB_PUSH_EVENT::acme/api');
  });

  it('falls back to slug when owner/repo are missing on a github_repo entry', () => {
    expect(triggerSignature('GITHUB_PUSH_EVENT', 'github_repo')).toBe('GITHUB_PUSH_EVENT');
  });

  it('matches active trigger signature using triggerConfig.owner/repo', () => {
    expect(
      activeTriggerSignature({
        id: 't1',
        slug: 'github_push_event',
        toolkit: 'github',
        connectionId: 'c1',
        triggerConfig: { owner: 'Acme', repo: 'API' },
      })
    ).toBe('GITHUB_PUSH_EVENT::acme/api');
  });

  it('falls back to slug when triggerConfig has no owner/repo', () => {
    expect(
      activeTriggerSignature({ id: 't2', slug: 'GMAIL_NEW', toolkit: 'gmail', connectionId: 'c1' })
    ).toBe('GMAIL_NEW');
  });
});

describe('<TriggerToggles>', () => {
  it('renders Loading then a list of available triggers', async () => {
    mockListAvailable.mockResolvedValue({
      triggers: [{ slug: 'GMAIL_NEW_GMAIL_MESSAGE', scope: 'static' }],
    });
    mockListTriggers.mockResolvedValue({ triggers: [] });

    render(<TriggerToggles toolkitSlug="gmail" toolkitName="Gmail" connectionId="c1" />);

    expect(screen.getByText('Loading…')).toBeInTheDocument();

    await waitFor(() =>
      expect(screen.getByLabelText(/Enable GMAIL_NEW_GMAIL_MESSAGE/)).toBeInTheDocument()
    );
    expect(mockListAvailable).toHaveBeenCalledWith('gmail', 'c1');
    expect(mockListTriggers).toHaveBeenCalledWith('gmail');
  });

  it('renders the empty state when no triggers are available', async () => {
    mockListAvailable.mockResolvedValue({ triggers: [] });
    mockListTriggers.mockResolvedValue({ triggers: [] });

    render(<TriggerToggles toolkitSlug="notion" toolkitName="Notion" connectionId="c1" />);

    await waitFor(() =>
      expect(
        screen.getByText('No triggers are currently available for Notion.')
      ).toBeInTheDocument()
    );
  });

  it('shows a load error when both lists fail', async () => {
    mockListAvailable.mockRejectedValue(new Error('boom'));
    mockListTriggers.mockResolvedValue({ triggers: [] });

    render(<TriggerToggles toolkitSlug="gmail" toolkitName="Gmail" connectionId="c1" />);

    await waitFor(() =>
      expect(screen.getByText(/Couldn't load triggers: boom/)).toBeInTheDocument()
    );
  });

  it('marks a trigger as enabled when present in active list (matched by signature)', async () => {
    mockListAvailable.mockResolvedValue({
      triggers: [{ slug: 'GMAIL_NEW_GMAIL_MESSAGE', scope: 'static' }],
    });
    mockListTriggers.mockResolvedValue({
      triggers: [
        { id: 't1', slug: 'GMAIL_NEW_GMAIL_MESSAGE', toolkit: 'gmail', connectionId: 'c1' },
      ],
    });

    render(<TriggerToggles toolkitSlug="gmail" toolkitName="Gmail" connectionId="c1" />);

    const sw = await screen.findByLabelText(/Disable GMAIL_NEW_GMAIL_MESSAGE/);
    expect(sw).toHaveAttribute('aria-checked', 'true');
  });

  it('ignores active triggers attached to a different connection', async () => {
    mockListAvailable.mockResolvedValue({
      triggers: [{ slug: 'GMAIL_NEW_GMAIL_MESSAGE', scope: 'static' }],
    });
    mockListTriggers.mockResolvedValue({
      triggers: [
        { id: 't1', slug: 'GMAIL_NEW_GMAIL_MESSAGE', toolkit: 'gmail', connectionId: 'OTHER' },
      ],
    });

    render(<TriggerToggles toolkitSlug="gmail" toolkitName="Gmail" connectionId="c1" />);

    const sw = await screen.findByLabelText(/Enable GMAIL_NEW_GMAIL_MESSAGE/);
    expect(sw).toHaveAttribute('aria-checked', 'false');
  });

  it('disables and shows a hint for static triggers that require config', async () => {
    mockListAvailable.mockResolvedValue({
      triggers: [{ slug: 'SLACK_NEW_MESSAGE', scope: 'static', requiredConfigKeys: ['channel'] }],
    });
    mockListTriggers.mockResolvedValue({ triggers: [] });

    render(<TriggerToggles toolkitSlug="slack" toolkitName="Slack" connectionId="c1" />);

    const sw = await screen.findByLabelText(/Enable SLACK_NEW_MESSAGE/);
    expect(sw).toBeDisabled();
    expect(screen.getByText('Needs configuration')).toBeInTheDocument();
  });

  it('enables a trigger via enableTrigger and flips the toggle on', async () => {
    mockListAvailable.mockResolvedValue({
      triggers: [
        { slug: 'GMAIL_NEW_GMAIL_MESSAGE', scope: 'static', defaultConfig: { labelIds: 'INBOX' } },
      ],
    });
    mockListTriggers.mockResolvedValue({ triggers: [] });
    mockEnable.mockResolvedValue({
      triggerId: 'ti_1',
      slug: 'GMAIL_NEW_GMAIL_MESSAGE',
      connectionId: 'c1',
    });

    render(<TriggerToggles toolkitSlug="gmail" toolkitName="Gmail" connectionId="c1" />);

    const sw = await screen.findByLabelText(/Enable GMAIL_NEW_GMAIL_MESSAGE/);
    fireEvent.click(sw);

    await waitFor(() =>
      expect(screen.getByLabelText(/Disable GMAIL_NEW_GMAIL_MESSAGE/)).toHaveAttribute(
        'aria-checked',
        'true'
      )
    );
    expect(mockEnable).toHaveBeenCalledWith('c1', 'GMAIL_NEW_GMAIL_MESSAGE', { labelIds: 'INBOX' });
  });

  it('renders github_repo entries with owner/repo label and forwards repo as triggerConfig on enable', async () => {
    mockListAvailable.mockResolvedValue({
      triggers: [
        {
          slug: 'GITHUB_PUSH_EVENT',
          scope: 'github_repo',
          repo: { owner: 'acme', repo: 'api' },
          defaultConfig: { owner: 'acme', repo: 'api' },
        },
      ],
    });
    mockListTriggers.mockResolvedValue({ triggers: [] });
    mockEnable.mockResolvedValue({
      triggerId: 'ti_g',
      slug: 'GITHUB_PUSH_EVENT',
      connectionId: 'c1',
    });

    render(<TriggerToggles toolkitSlug="github" toolkitName="GitHub" connectionId="c1" />);

    expect(await screen.findByText('acme/api')).toBeInTheDocument();
    fireEvent.click(screen.getByLabelText(/Enable GITHUB_PUSH_EVENT/));

    await waitFor(() => expect(mockEnable).toHaveBeenCalled());
    expect(mockEnable).toHaveBeenCalledWith('c1', 'GITHUB_PUSH_EVENT', {
      owner: 'acme',
      repo: 'api',
    });
  });

  it('disables a trigger via disableTrigger and flips the toggle off', async () => {
    mockListAvailable.mockResolvedValue({
      triggers: [{ slug: 'GMAIL_NEW_GMAIL_MESSAGE', scope: 'static' }],
    });
    mockListTriggers.mockResolvedValue({
      triggers: [
        { id: 't1', slug: 'GMAIL_NEW_GMAIL_MESSAGE', toolkit: 'gmail', connectionId: 'c1' },
      ],
    });
    mockDisable.mockResolvedValue({ deleted: true });

    render(<TriggerToggles toolkitSlug="gmail" toolkitName="Gmail" connectionId="c1" />);

    const sw = await screen.findByLabelText(/Disable GMAIL_NEW_GMAIL_MESSAGE/);
    fireEvent.click(sw);

    await waitFor(() =>
      expect(screen.getByLabelText(/Enable GMAIL_NEW_GMAIL_MESSAGE/)).toHaveAttribute(
        'aria-checked',
        'false'
      )
    );
    expect(mockDisable).toHaveBeenCalledWith('t1');
  });

  it('surfaces an error message when enableTrigger fails', async () => {
    mockListAvailable.mockResolvedValue({
      triggers: [{ slug: 'GMAIL_NEW_GMAIL_MESSAGE', scope: 'static' }],
    });
    mockListTriggers.mockResolvedValue({ triggers: [] });
    mockEnable.mockRejectedValue(new Error('upstream 500'));

    render(<TriggerToggles toolkitSlug="gmail" toolkitName="Gmail" connectionId="c1" />);

    fireEvent.click(await screen.findByLabelText(/Enable GMAIL_NEW_GMAIL_MESSAGE/));

    await waitFor(() =>
      expect(
        screen.getByText(/Enable failed for GMAIL_NEW_GMAIL_MESSAGE: upstream 500/)
      ).toBeInTheDocument()
    );
  });
});
