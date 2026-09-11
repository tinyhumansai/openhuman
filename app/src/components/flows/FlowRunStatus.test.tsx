import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import type { FlowRunStatus as FlowRunStatusValue } from '../../services/api/flowsApi';
import {
  FLOW_RUN_STATUS_ACCENT,
  FLOW_RUN_STATUS_DOT,
  FLOW_RUN_STATUS_KEY,
  FlowRunStatus,
  flowRunStatusAccentClass,
  flowRunStatusDotClass,
  flowRunStatusLabel,
} from './FlowRunStatus';

const EXPECTED_STATUS_PRESENTATION: Record<
  FlowRunStatusValue,
  { accent: string; dot: string; key: string }
> = {
  running: {
    accent:
      'border-primary-200 bg-primary-50 text-primary-700 dark:border-primary-500/30 dark:bg-primary-500/10 dark:text-primary-300',
    dot: 'bg-primary-500 animate-pulse',
    key: 'flowRuns.status.running',
  },
  completed: {
    accent:
      'border-sage-200 bg-sage-50 text-sage-700 dark:border-sage-500/30 dark:bg-sage-500/10 dark:text-sage-300',
    dot: 'bg-sage-500',
    key: 'flowRuns.status.completed',
  },
  completed_with_warnings: {
    accent:
      'border-amber-200 bg-amber-50 text-amber-700 dark:border-amber-500/30 dark:bg-amber-500/10 dark:text-amber-300',
    dot: 'bg-amber-500',
    key: 'flowRuns.status.completed_with_warnings',
  },
  pending_approval: {
    accent:
      'border-amber-200 bg-amber-50 text-amber-700 dark:border-amber-500/30 dark:bg-amber-500/10 dark:text-amber-300',
    dot: 'bg-amber-500 animate-pulse',
    key: 'flowRuns.status.pending_approval',
  },
  failed: {
    accent:
      'border-coral-200 bg-coral-50 text-coral-700 dark:border-coral-500/30 dark:bg-coral-500/10 dark:text-coral-300',
    dot: 'bg-coral-500',
    key: 'flowRuns.status.failed',
  },
  cancelled: {
    accent: 'border-line bg-surface-muted text-content-secondary',
    dot: 'bg-surface-strong',
    key: 'flowRuns.status.cancelled',
  },
  interrupted: {
    accent:
      'border-amber-200 bg-amber-50 text-amber-700 dark:border-amber-500/30 dark:bg-amber-500/10 dark:text-amber-300',
    dot: 'bg-amber-500',
    key: 'flowRuns.status.interrupted',
  },
};

describe('FlowRunStatus', () => {
  it.each(Object.entries(EXPECTED_STATUS_PRESENTATION))(
    'pins %s to the existing accent, dot, and localization key',
    (status, expected) => {
      expect(FLOW_RUN_STATUS_ACCENT[status as FlowRunStatusValue]).toBe(expected.accent);
      expect(FLOW_RUN_STATUS_DOT[status as FlowRunStatusValue]).toBe(expected.dot);
      expect(FLOW_RUN_STATUS_KEY[status as FlowRunStatusValue]).toBe(expected.key);
    }
  );

  it('renders the supplied localized label as a badge by default', () => {
    render(
      <FlowRunStatus
        status="completed_with_warnings"
        label="Completed with localized warnings"
        className="consumer-badge-class"
        testId="status"
      />
    );

    const badge = screen.getByTestId('status');
    expect(badge).toHaveTextContent('Completed with localized warnings');
    expect(badge).toHaveClass(
      'inline-flex',
      'rounded-full',
      'border',
      'consumer-badge-class',
      'bg-amber-50'
    );
    expect(badge).not.toHaveAttribute('aria-hidden');
  });

  it('renders a decorative dot without exposing the label', () => {
    render(
      <FlowRunStatus
        status="pending_approval"
        label="Awaiting localized approval"
        presentation="dot"
        className="consumer-dot-class"
        testId="status-dot"
      />
    );

    const dot = screen.getByTestId('status-dot');
    expect(dot).toBeEmptyDOMElement();
    expect(dot).toHaveAttribute('aria-hidden', 'true');
    expect(dot).toHaveClass(
      'h-2',
      'w-2',
      'rounded-full',
      'consumer-dot-class',
      'bg-amber-500',
      'animate-pulse'
    );
  });

  // F-m8: run payloads are cast, never validated, so a future/unknown
  // `FlowRunStatus` string must fall back rather than index these maps with
  // `undefined` — mirroring WorkflowRunsPage.tsx's existing fallback pattern.
  describe('unrecognized wire status (F-m8)', () => {
    const unknown = 'archived' as FlowRunStatusValue;

    it('flowRunStatusLabel falls back to a humanized status instead of an undefined key', () => {
      const t = (key: string, fallback?: string) => fallback ?? `MISSING:${key}`;
      expect(flowRunStatusLabel(unknown, t)).toBe('archived');
      // A recognized status still resolves through the real key + t().
      expect(flowRunStatusLabel('completed', t)).toBe(t('flowRuns.status.completed', 'completed'));
    });

    it('flowRunStatusLabel humanizes underscores in the fallback', () => {
      const t = (_key: string, fallback?: string) => fallback ?? '';
      expect(flowRunStatusLabel('archived_forever' as FlowRunStatusValue, t)).toBe(
        'archived forever'
      );
    });

    it('flowRunStatusAccentClass / flowRunStatusDotClass fall back to a neutral class', () => {
      expect(flowRunStatusAccentClass(unknown)).toBe(
        'border-line bg-surface-muted text-content-secondary'
      );
      expect(flowRunStatusDotClass(unknown)).toBe('bg-surface-strong');
    });

    it('FlowRunStatus renders without an "undefined" class for an unrecognized status', () => {
      render(<FlowRunStatus status={unknown} label="archived" testId="status-unknown" />);
      const badge = screen.getByTestId('status-unknown');
      expect(badge).toHaveTextContent('archived');
      expect(badge.className).not.toContain('undefined');
    });
  });
});
