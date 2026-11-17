import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import {
  gaugeAngleFor,
  ReasoningEffortPicker,
  toReasoningEffortChoice,
} from './ReasoningEffortPicker';

vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

describe('ReasoningEffortPicker', () => {
  it('offers every thinking level and reports the pick', () => {
    const onChange = vi.fn();
    render(<ReasoningEffortPicker value="default" onChange={onChange} />);

    const select = screen.getByTestId('composer-reasoning-effort') as HTMLSelectElement;
    expect(select).toHaveAccessibleName('composer.reasoning.label');
    expect(Array.from(select.options).map(o => o.value)).toEqual([
      'default',
      'none',
      'minimal',
      'low',
      'medium',
      'high',
      'xhigh',
    ]);

    fireEvent.change(select, { target: { value: 'high' } });
    expect(onChange).toHaveBeenCalledWith('high');
  });

  it('shows the current value and can be disabled', () => {
    render(<ReasoningEffortPicker value="none" onChange={vi.fn()} disabled />);
    const select = screen.getByTestId('composer-reasoning-effort') as HTMLSelectElement;
    expect(select.value).toBe('none');
    expect(select).toBeDisabled();
  });
});

describe('gaugeAngleFor', () => {
  it('sweeps from off to max and points up for the provider default', () => {
    expect(gaugeAngleFor('default')).toBe(0);
    expect(gaugeAngleFor('none')).toBe(-120);
    expect(gaugeAngleFor('xhigh')).toBe(120);
    expect(gaugeAngleFor('low')).toBeLessThan(gaugeAngleFor('high'));
  });
});

describe('ReasoningEffortPicker per-model tooltip', () => {
  it('names the model the level is remembered for', () => {
    render(<ReasoningEffortPicker value="high" onChange={vi.fn()} modelLabel="opus" />);
    expect(screen.getByTestId('composer-reasoning-effort')).toHaveAttribute(
      'title',
      'composer.reasoning.forModel'
    );
  });
});

describe('toReasoningEffortChoice', () => {
  it('normalizes config values and aliases', () => {
    expect(toReasoningEffortChoice('HIGH')).toBe('high');
    expect(toReasoningEffortChoice('off')).toBe('none');
    expect(toReasoningEffortChoice('max')).toBe('xhigh');
    expect(toReasoningEffortChoice('minimal')).toBe('minimal');
    expect(toReasoningEffortChoice('turbo')).toBe('default');
    expect(toReasoningEffortChoice(null)).toBe('default');
    expect(toReasoningEffortChoice('')).toBe('default');
  });
});
