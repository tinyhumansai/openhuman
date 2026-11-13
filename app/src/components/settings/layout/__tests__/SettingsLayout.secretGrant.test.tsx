/**
 * Saving an API key from Settings needs the native secret master grant
 * (`native_secret_grant`). That RPC has no chat thread behind it, so the core
 * parks the grant as an unrouted pending approval. These tests render the real
 * `SettingsLayout` route shell and assert the grant reaches the user there —
 * on whichever settings panel did the save — and that the decision goes
 * through the shared approval RPC.
 */
import { combineReducers, configureStore } from '@reduxjs/toolkit';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Provider } from 'react-redux';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { resetFlowPendingApprovalsStoreForTests } from '../../../../hooks/flowPendingApprovalsStore';
import {
  decideApproval,
  fetchPendingApprovals,
  type PendingApproval,
} from '../../../../services/api/approvalApi';
import chatRuntimeReducer from '../../../../store/chatRuntimeSlice';
import { SidebarSlotOutlet, SidebarSlotProvider } from '../../../layout/shell/SidebarSlot';
import SettingsLayout from '../SettingsLayout';

vi.mock('../../../../services/api/approvalApi', async importOriginal => {
  const actual = await importOriginal<typeof import('../../../../services/api/approvalApi')>();
  return { ...actual, fetchPendingApprovals: vi.fn(), decideApproval: vi.fn() };
});
vi.mock('../SettingsSidebar', () => ({ default: () => null }));

const SECRET_GRANT: PendingApproval = {
  request_id: 'req-secret-grant-1',
  tool_name: 'native_secret_grant',
  action_summary: 'Unlock configuration secrets',
  args_redacted: { registration_id: 'opaque' },
  created_at: '2026-11-07T10:00:00.000Z',
  expires_at: '2099-11-07T10:10:00.000Z',
};

const BACKGROUND_PARK: PendingApproval = {
  request_id: 'req-triage-1',
  tool_name: 'triage.escalate',
  action_summary: 'triage::ESCALATE target=orchestrator',
  args_redacted: { action: 'escalate' },
  created_at: '2026-11-07T10:00:00.000Z',
  expires_at: '2099-11-07T10:10:00.000Z',
};

function renderSettings() {
  const store = configureStore({ reducer: combineReducers({ chatRuntime: chatRuntimeReducer }) });
  render(
    <Provider store={store}>
      <MemoryRouter initialEntries={['/settings/ai']}>
        <SidebarSlotProvider>
          <SidebarSlotOutlet />
          <Routes>
            <Route path="/settings" element={<SettingsLayout />}>
              <Route path="ai" element={<div data-testid="ai-panel">AI panel</div>} />
            </Route>
          </Routes>
        </SidebarSlotProvider>
      </MemoryRouter>
    </Provider>
  );
}

beforeEach(() => {
  resetFlowPendingApprovalsStoreForTests();
  vi.mocked(decideApproval).mockResolvedValue(undefined as never);
});

afterEach(() => {
  resetFlowPendingApprovalsStoreForTests();
  vi.clearAllMocks();
});

describe('the secret grant prompt on Settings', () => {
  it('shows a parked secret grant above the settings panel and approves it once', async () => {
    vi.mocked(fetchPendingApprovals).mockResolvedValue([SECRET_GRANT]);
    renderSettings();

    const card = await screen.findByTestId('secret-grant-approval-card', undefined, {
      timeout: 5000,
    });
    expect(card).toHaveTextContent('Allow OpenHuman to unlock your saved secrets?');
    expect(screen.getByTestId('ai-panel')).toBeInTheDocument();

    await userEvent.click(screen.getByRole('button', { name: 'Approve' }));
    await waitFor(() =>
      expect(vi.mocked(decideApproval)).toHaveBeenCalledWith('req-secret-grant-1', 'approve_once')
    );
  });

  it('records a denial through the shared approval RPC', async () => {
    vi.mocked(fetchPendingApprovals).mockResolvedValue([SECRET_GRANT]);
    renderSettings();
    await screen.findByTestId('secret-grant-approval-card', undefined, { timeout: 5000 });

    await userEvent.click(screen.getByRole('button', { name: 'Deny' }));
    await waitFor(() =>
      expect(vi.mocked(decideApproval)).toHaveBeenCalledWith('req-secret-grant-1', 'deny')
    );
  });

  it('leaves other background approvals to the chat deck', async () => {
    vi.mocked(fetchPendingApprovals).mockResolvedValue([BACKGROUND_PARK]);
    renderSettings();

    await waitFor(() => expect(vi.mocked(fetchPendingApprovals)).toHaveBeenCalled());
    expect(screen.queryByTestId('secret-grant-approval-card')).toBeNull();
  });
});
