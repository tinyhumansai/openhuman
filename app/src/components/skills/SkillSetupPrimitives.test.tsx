/**
 * SkillSetupPrimitives — smoke coverage for the ui/ primitive migration.
 *
 * `SetupSuccess` is the only interactive primitive in the file: two stacked
 * actions that must stay independently wired after the raw `<button>`s became
 * `Button`s.
 */
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { SetupSettingRow, SetupSuccess } from './SkillSetupPrimitives';

describe('SetupSuccess', () => {
  it('wires each action to its own handler', () => {
    const onSettings = vi.fn();
    const onFinish = vi.fn();
    render(
      <SetupSuccess
        title="All set"
        description="Your skill is ready"
        settingsLabel="Open settings"
        finishLabel="Done"
        onSettings={onSettings}
        onFinish={onFinish}
      />
    );

    fireEvent.click(screen.getByRole('button', { name: 'Open settings' }));
    expect(onSettings).toHaveBeenCalledTimes(1);
    expect(onFinish).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole('button', { name: 'Done' }));
    expect(onFinish).toHaveBeenCalledTimes(1);
  });

  it('renders both actions as secondary Buttons', () => {
    render(
      <SetupSuccess
        title="All set"
        description="Your skill is ready"
        settingsLabel="Open settings"
        finishLabel="Done"
        onSettings={vi.fn()}
        onFinish={vi.fn()}
      />
    );

    for (const name of ['Open settings', 'Done']) {
      const button = screen.getByRole('button', { name });
      expect(button).toHaveAttribute('data-slot', 'button');
      expect(button).toHaveAttribute('data-variant', 'secondary');
    }
  });
});

describe('SetupSettingRow', () => {
  it('renders its label and value', () => {
    render(<SetupSettingRow label="Model" value="gpt-5" mono />);
    expect(screen.getByText('Model')).toBeInTheDocument();
    expect(screen.getByText('gpt-5')).toBeInTheDocument();
  });
});
