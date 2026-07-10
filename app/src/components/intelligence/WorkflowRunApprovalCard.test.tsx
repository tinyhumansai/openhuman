import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { type WorkflowDefinition } from '../../services/api/workflowRunsApi';
import WorkflowRunApprovalCard from './WorkflowRunApprovalCard';

// i18n → echo the key so assertions can target stable strings.
vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

function def(overrides: Partial<WorkflowDefinition> = {}): WorkflowDefinition {
  return {
    id: 'parallel_research_cross_check',
    name: 'Parallel research',
    description: 'desc',
    phases: [],
    defaultConcurrency: 2,
    maxChildren: 8,
    safetyTier: 'read_only',
    ...overrides,
  };
}

describe('WorkflowRunApprovalCard', () => {
  it('renders one localized line per reason', () => {
    render(
      <WorkflowRunApprovalCard
        definition={def()}
        reasons={['high_children', 'high_concurrency']}
        onApprove={vi.fn()}
        onCancel={vi.fn()}
      />
    );
    const reasons = screen.getByTestId('workflow-approval-reasons');
    expect(reasons.querySelectorAll('li')).toHaveLength(2);
    expect(screen.getByText('orchestration.approval.reason.children')).toBeInTheDocument();
    expect(screen.getByText('orchestration.approval.reason.concurrency')).toBeInTheDocument();
  });

  it('shows the concrete cost facts (tier / concurrency / max children)', () => {
    render(
      <WorkflowRunApprovalCard
        definition={def({ defaultConcurrency: 3, maxChildren: 8 })}
        reasons={['high_children']}
        onApprove={vi.fn()}
        onCancel={vi.fn()}
      />
    );
    expect(screen.getByText('orchestration.tier.readOnly')).toBeInTheDocument();
    expect(screen.getByText('3')).toBeInTheDocument();
    expect(screen.getByText('8')).toBeInTheDocument();
  });

  it('fires onApprove when "Approve & start" is clicked', () => {
    const onApprove = vi.fn();
    render(
      <WorkflowRunApprovalCard
        definition={def()}
        reasons={['high_children']}
        onApprove={onApprove}
        onCancel={vi.fn()}
      />
    );
    fireEvent.click(screen.getByTestId('workflow-approval-approve'));
    expect(onApprove).toHaveBeenCalledTimes(1);
  });

  it('fires onCancel when "Cancel" is clicked', () => {
    const onCancel = vi.fn();
    render(
      <WorkflowRunApprovalCard
        definition={def()}
        reasons={['high_children']}
        onApprove={vi.fn()}
        onCancel={onCancel}
      />
    );
    fireEvent.click(screen.getByTestId('workflow-approval-cancel'));
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it('uses an opaque warning surface so thread text does not show through (#3783)', () => {
    render(
      <WorkflowRunApprovalCard
        definition={def()}
        reasons={['high_children']}
        onApprove={vi.fn()}
        onCancel={vi.fn()}
      />
    );
    const card = screen.getByTestId('workflow-approval-card');
    expect(card).toHaveClass('bg-amber-50');
    expect(card).toHaveClass('dark:bg-amber-950');
    // No fractional-opacity background that would let thread text bleed through.
    expect(card.className).not.toMatch(/\bbg-[^\s/]+\/\d+/);
    expect(card.className).not.toMatch(/\bdark:bg-[^\s/]+\/\d+/);
  });

  it('disables both buttons and shows the starting label while a start is in flight', () => {
    render(
      <WorkflowRunApprovalCard
        definition={def()}
        reasons={['high_children']}
        starting
        onApprove={vi.fn()}
        onCancel={vi.fn()}
      />
    );
    expect(screen.getByTestId('workflow-approval-approve')).toBeDisabled();
    expect(screen.getByTestId('workflow-approval-cancel')).toBeDisabled();
    expect(screen.getByText('orchestration.approval.starting')).toBeInTheDocument();
  });
});
