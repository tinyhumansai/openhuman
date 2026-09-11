import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { FeedbackStatus } from '../../types/feedback';
import FeedbackStatusBadge from './FeedbackStatusBadge';

vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

describe('<FeedbackStatusBadge />', () => {
  it('renders the label and styling for a known status', () => {
    render(<FeedbackStatusBadge status="completed" />);
    const badge = screen.getByText('feedback.status.completed');
    expect(badge.className).toContain('bg-sage-500/10');
  });

  it('renders a status the build does not know about instead of throwing', () => {
    // `FeedbackStatus` is a compile-time union, but `status` comes from the API
    // and nothing validates it there. An unknown value used to make the style
    // lookup undefined and `style.pill` throw, which took the whole feedback
    // page down over a single row's badge.
    const unknown = 'in_review' as FeedbackStatus;
    expect(() => render(<FeedbackStatusBadge status={unknown} />)).not.toThrow();
    expect(screen.getByText('in_review')).toBeInTheDocument();
  });

  it('does not dress an unknown status as a known one', () => {
    const unknown = 'in_review' as FeedbackStatus;
    render(<FeedbackStatusBadge status={unknown} />);
    const badge = screen.getByText('in_review');
    expect(badge.className).toContain('text-content-muted');
    expect(badge.className).not.toContain('bg-sage-500/10');
  });

  it('survives a missing status', () => {
    const missing = undefined as unknown as FeedbackStatus;
    expect(() => render(<FeedbackStatusBadge status={missing} />)).not.toThrow();
  });
});
