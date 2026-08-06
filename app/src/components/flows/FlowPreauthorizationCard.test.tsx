import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { ApprovalManifestEntry } from '../../services/api/flowsApi';
import { FlowPreauthorizationCard, FlowPreauthorizationOverlay } from './FlowPreauthorizationCard';

const ENTRIES: ApprovalManifestEntry[] = [
  {
    kind: 'approvable',
    node_id: 'n1',
    tool_name: 'flows_http_request',
    label: 'Call https://api.example.com',
    class: 'Network',
  },
  {
    kind: 'approvable',
    node_id: 'n2',
    tool_name: 'GMAIL_SEND_EMAIL',
    label: 'Use GMAIL_SEND_EMAIL',
    class: 'Network',
  },
  { kind: 'blocked', node_id: 'n3', tool_name: 'flows_code', label: 'Run sandboxed code' },
  { kind: 'dynamic', node_id: 'n4', label: 'Tool chosen at run time' },
  { kind: 'agent', node_id: 'n5', label: 'AI step' },
];

describe('FlowPreauthorizationCard', () => {
  it('renders every manifest row with its kind-specific hint', () => {
    render(
      <FlowPreauthorizationCard
        entries={ENTRIES}
        busy={false}
        onApproveAll={vi.fn()}
        onDeny={vi.fn()}
      />
    );

    expect(
      screen.getByRole('alertdialog', { name: 'Allow this workflow to act?' })
    ).toHaveAttribute('data-testid', 'flow-preauthorization-card');
    expect(screen.getAllByTestId('flow-preauth-row-approvable')).toHaveLength(2);
    expect(screen.getByText('Call https://api.example.com')).toBeInTheDocument();
    // Informational rows carry their hints; approvable rows carry none.
    expect(screen.getByText('Blocked by your agent access settings.')).toBeInTheDocument();
    expect(
      screen.getByText('Chosen while the workflow runs; it will ask you if needed.')
    ).toBeInTheDocument();
    expect(
      screen.getByText('This AI step may ask separately for its own actions.')
    ).toBeInTheDocument();
  });

  it('exposes exactly two actions: Approve all and Deny', () => {
    const onApproveAll = vi.fn();
    const onDeny = vi.fn();
    render(
      <FlowPreauthorizationCard
        entries={ENTRIES}
        busy={false}
        onApproveAll={onApproveAll}
        onDeny={onDeny}
      />
    );

    const buttons = screen.getAllByRole('button');
    expect(buttons).toHaveLength(2);

    fireEvent.click(screen.getByRole('button', { name: 'Approve all' }));
    expect(onApproveAll).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByRole('button', { name: 'Deny' }));
    expect(onDeny).toHaveBeenCalledTimes(1);
  });

  it('disables both actions and shows the busy label while granting', () => {
    render(
      <FlowPreauthorizationCard entries={ENTRIES} busy onApproveAll={vi.fn()} onDeny={vi.fn()} />
    );

    expect(screen.getByRole('button', { name: 'Approving…' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Deny' })).toBeDisabled();
  });

  it('renders the error message when provided', () => {
    render(
      <FlowPreauthorizationCard
        entries={ENTRIES}
        busy={false}
        errorMsg="Could not save the approvals. Please try again."
        onApproveAll={vi.fn()}
        onDeny={vi.fn()}
      />
    );

    expect(
      screen.getByText(/Could not save the approvals\. Please try again\./)
    ).toBeInTheDocument();
  });
});

describe('FlowPreauthorizationCard blocked-only', () => {
  it("swaps the primary action to 'Enable anyway' when nothing is approvable", () => {
    render(
      <FlowPreauthorizationCard
        entries={[
          { kind: 'blocked', node_id: 'n1', tool_name: 'flows_http_request', label: 'Call API' },
        ]}
        busy={false}
        onApproveAll={vi.fn()}
        onDeny={vi.fn()}
      />
    );

    expect(screen.getByRole('button', { name: 'Enable anyway' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Approve all' })).not.toBeInTheDocument();
  });
});

describe('FlowPreauthorizationOverlay', () => {
  it('wraps the card in a full-screen overlay for page contexts', () => {
    render(
      <FlowPreauthorizationOverlay
        entries={ENTRIES}
        busy={false}
        onApproveAll={vi.fn()}
        onDeny={vi.fn()}
      />
    );

    expect(screen.getByTestId('flow-preauthorization-overlay')).toBeInTheDocument();
    expect(screen.getByTestId('flow-preauthorization-card')).toBeInTheDocument();
  });
});
