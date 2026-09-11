import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import PersonaGuidedFields from './PersonaGuidedFields';

// Pass-through translator so assertions can target the i18n keys directly.
vi.mock('../../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

const navigateToSettings = vi.fn();
vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({ navigateToSettings }),
}));

describe('<PersonaGuidedFields />', () => {
  it('renders the personality, voice and about text areas', () => {
    render(<PersonaGuidedFields value="" onChange={vi.fn()} />);
    expect(screen.getByTestId('persona-guided-personality')).toBeInTheDocument();
    expect(screen.getByTestId('persona-guided-voice')).toBeInTheDocument();
    expect(screen.getByTestId('persona-guided-about')).toBeInTheDocument();
  });

  it('emits the updated SOUL.md text when a field changes', () => {
    const onChange = vi.fn();
    render(<PersonaGuidedFields value="" onChange={onChange} />);
    fireEvent.change(screen.getByTestId('persona-guided-personality'), {
      target: { value: 'Curious and warm.' },
    });
    expect(onChange).toHaveBeenCalledWith(expect.stringContaining('Curious and warm.'));
  });

  it('navigates to agent access from the security link button', () => {
    render(<PersonaGuidedFields value="" onChange={vi.fn()} />);
    const link = screen.getByTestId('persona-guided-agent-access');
    expect(link).toHaveAttribute('data-slot', 'button');
    fireEvent.click(link);
    expect(navigateToSettings).toHaveBeenCalledWith('agent-access');
  });

  it('disables the fields and template picker when disabled', () => {
    render(<PersonaGuidedFields value="" onChange={vi.fn()} disabled />);
    expect(screen.getByTestId('persona-guided-personality')).toBeDisabled();
  });
});
