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

  // F-m7: a hand-written step outside buildCron's clamp range (e.g. "every 90
  // minutes") must open in the advanced field, exactly like any other cron
  // the visual builder doesn't model — not the visual "minutes" editor with an
  // out-of-range interval, whose next unrelated patch() would silently
  // rewrite it to */59.
  it('opens the advanced cron field for an out-of-range step instead of the visual minutes editor', () => {
    setup('*/90 * * * *');
    expect(screen.getByTestId('sched-cron')).toHaveValue('*/90 * * * *');
    expect(screen.queryByTestId('sched-interval')).not.toBeInTheDocument();
  });

  it('leaves an out-of-range custom cron untouched by onChange on mount (no silent rewrite)', () => {
    const { onChange } = setup('*/90 * * * *');
    // Only the mount-seed effect can call onChange unprompted, and it's
    // guarded on an empty value — an out-of-range but non-empty value must
    // never be silently recompiled to */59.
    expect(onChange).not.toHaveBeenCalled();
  });
});
