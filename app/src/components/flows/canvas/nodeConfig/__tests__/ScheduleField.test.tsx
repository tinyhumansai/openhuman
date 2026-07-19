/**
 * Behavior tests for the friendly schedule builder. Asserts it compiles the
 * visual controls (frequency, interval, weekday toggles) to a cron string, shows
 * a live plain-English summary, seeds a default when empty, and round-trips a
 * custom cron through the advanced text field. `useT()` falls back to the
 * bundled English map with no provider mounted (same as the sibling tests).
 */
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { ScheduleField } from '../ScheduleField';

function setup(value = '*/5 * * * *') {
  const onChange = vi.fn();
  render(<ScheduleField value={value} onChange={onChange} testId="sched" />);
  return { onChange };
}

describe('ScheduleField', () => {
  it('renders a plain-English summary of the current cron', () => {
    setup('*/5 * * * 3');
    expect(screen.getByTestId('sched-summary')).toHaveTextContent('Every 5 minutes on Wed');
  });

  it('seeds a default cron when mounted empty', () => {
    const { onChange } = setup('');
    // Mount effect writes the default daily-9am schedule.
    expect(onChange).toHaveBeenCalledWith('0 9 * * *');
  });

  it('recompiles the cron when the interval changes', () => {
    const { onChange } = setup('*/5 * * * *');
    fireEvent.change(screen.getByTestId('sched-interval'), { target: { value: '10' } });
    expect(onChange).toHaveBeenLastCalledWith('*/10 * * * *');
  });

  it('recompiles the cron when the frequency changes to daily', () => {
    const { onChange } = setup('*/5 * * * *');
    fireEvent.change(screen.getByTestId('sched-freq'), { target: { value: 'daily' } });
    // Default daily time (09:00), keeping "every day".
    expect(onChange).toHaveBeenLastCalledWith('0 9 * * *');
  });

  it('toggles a weekday into the cron', () => {
    const { onChange } = setup('*/5 * * * *');
    // Day index 3 = Wednesday.
    fireEvent.click(screen.getByTestId('sched-day-3'));
    expect(onChange).toHaveBeenLastCalledWith('*/5 * * * 3');
  });

  it('opens the advanced cron field for an unmodellable expression', () => {
    const { onChange } = setup('0 9 1 * *'); // day-of-month set → advanced
    const cron = screen.getByTestId('sched-cron');
    expect(cron).toHaveValue('0 9 1 * *');
    fireEvent.change(cron, { target: { value: '15 3 * * *' } });
    expect(onChange).toHaveBeenLastCalledWith('15 3 * * *');
  });
});
