import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import type { InstanceStatus } from '../../lib/orchestration/orchestrationClient';
import InstanceStatusDot from './InstanceStatusDot';

describe('InstanceStatusDot', () => {
  it.each<InstanceStatus>(['running', 'idle', 'waiting-approval', 'errored', 'stopped'])(
    'tags the %s status and pulses only when running',
    status => {
      render(<InstanceStatusDot status={status} label={status} />);
      const dot = screen.getByTestId('instance-status-dot');
      expect(dot).toHaveAttribute('data-status', status);
      expect(dot.className.includes('animate-pulse')).toBe(status === 'running');
    }
  );

  it('uses the status value as the accessible name when no label is given', () => {
    render(<InstanceStatusDot status="errored" />);
    expect(screen.getByTestId('instance-status-dot')).toHaveAttribute('aria-label', 'errored');
  });

  it('prefers a provided translated label', () => {
    render(<InstanceStatusDot status="idle" label="Inactivo" />);
    expect(screen.getByTestId('instance-status-dot')).toHaveAttribute('aria-label', 'Inactivo');
  });
});
