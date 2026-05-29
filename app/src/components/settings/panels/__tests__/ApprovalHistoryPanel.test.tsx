import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  type ApprovalAuditEntry,
  fetchRecentApprovalDecisions,
} from '../../../../services/api/approvalApi';
import { renderWithProviders } from '../../../../test/test-utils';
import ApprovalHistoryPanel from '../ApprovalHistoryPanel';

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateBack: vi.fn(),
    navigateToSettings: vi.fn(),
    breadcrumbs: [],
  }),
}));

vi.mock('../../../../services/api/approvalApi', () => ({ fetchRecentApprovalDecisions: vi.fn() }));

const mockFetch = vi.mocked(fetchRecentApprovalDecisions);

const auditRow = (overrides: Partial<ApprovalAuditEntry> = {}): ApprovalAuditEntry => ({
  request_id: 'req-1',
  tool_name: 'shell',
  action_summary: 'run ls -la',
  args_redacted: {},
  session_id: 'sess-1',
  created_at: '2026-05-29T10:00:00Z',
  expires_at: null,
  decided_at: '2026-05-29T10:00:05Z',
  decision: 'approve_once',
  ...overrides,
});

describe('ApprovalHistoryPanel', () => {
  beforeEach(() => {
    mockFetch.mockReset();
  });

  it('renders the loaded list of decided approvals', async () => {
    mockFetch.mockResolvedValueOnce([
      auditRow({ request_id: 'a', tool_name: 'shell', decision: 'approve_once' }),
      auditRow({ request_id: 'b', tool_name: 'curl', decision: 'deny' }),
    ]);

    renderWithProviders(<ApprovalHistoryPanel />, {
      initialEntries: ['/settings/approval-history'],
    });

    await screen.findByTestId('approval-history-list');
    const rows = screen.getAllByTestId('approval-history-row');
    expect(rows).toHaveLength(2);
    expect(screen.getByText('shell')).toBeInTheDocument();
    expect(screen.getByText('curl')).toBeInTheDocument();
  });

  it('renders a decision badge per row', async () => {
    mockFetch.mockResolvedValueOnce([
      auditRow({ request_id: 'a', decision: 'approve_always_for_tool' }),
      auditRow({ request_id: 'b', decision: 'deny' }),
    ]);

    renderWithProviders(<ApprovalHistoryPanel />, {
      initialEntries: ['/settings/approval-history'],
    });

    await screen.findByTestId('approval-history-list');
    expect(
      screen.getByTestId('approval-history-decision-approve_always_for_tool')
    ).toBeInTheDocument();
    expect(screen.getByTestId('approval-history-decision-deny')).toBeInTheDocument();
  });

  it('renders the empty state when there are no decisions', async () => {
    mockFetch.mockResolvedValueOnce([]);

    renderWithProviders(<ApprovalHistoryPanel />, {
      initialEntries: ['/settings/approval-history'],
    });

    await screen.findByTestId('approval-history-empty');
    expect(screen.queryByTestId('approval-history-list')).not.toBeInTheDocument();
  });

  it('renders a localized error state when the fetch rejects', async () => {
    mockFetch.mockRejectedValueOnce(new Error('boom'));

    renderWithProviders(<ApprovalHistoryPanel />, {
      initialEntries: ['/settings/approval-history'],
    });

    const err = await screen.findByTestId('approval-history-error');
    // Raw backend text must never leak into the UI.
    expect(err.textContent).not.toContain('boom');
  });

  it('refetches when the Refresh button is clicked', async () => {
    mockFetch.mockResolvedValue([auditRow()]);

    renderWithProviders(<ApprovalHistoryPanel />, {
      initialEntries: ['/settings/approval-history'],
    });

    await screen.findByTestId('approval-history-list');
    expect(mockFetch).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByTestId('approval-history-refresh'));
    await waitFor(() => expect(mockFetch).toHaveBeenCalledTimes(2));
  });
});
