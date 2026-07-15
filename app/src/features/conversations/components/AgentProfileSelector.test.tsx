import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { AgentProfile } from '../../../types/agentProfile';
import { AgentProfileSelector } from './AgentProfileSelector';

vi.mock('../../../lib/i18n/I18nContext', () => ({
  useT: () => ({ t: (key: string) => key }),
}));

const customProfile: AgentProfile = {
  id: 'research',
  name: 'Research',
  builtIn: false,
  description: '',
  agentId: 'orchestrator',
};

describe('AgentProfileSelector', () => {
  it('renders a button for each non-built-in profile and dispatches selection on click', () => {
    const onSelect = vi.fn();
    render(
      <AgentProfileSelector
        profiles={[customProfile]}
        selectedProfileId="default"
        locale="en"
        onSelect={onSelect}
      />
    );

    const button = screen.getByRole('radio', { name: 'Research' });
    expect(button).toHaveAttribute('aria-checked', 'false');
    expect(button).toHaveAttribute('data-analytics-id', 'chat-header-mode-research');

    fireEvent.click(button);
    expect(onSelect).toHaveBeenCalledWith('research');
  });

  it('does not render buttons for built-in profiles', () => {
    const onSelect = vi.fn();
    render(
      <AgentProfileSelector
        profiles={[
          { id: 'default', name: 'Default', builtIn: true, description: '', agentId: 'orchestrator' },
          { id: 'reasoning', name: 'Reasoning', builtIn: true, description: '', agentId: 'orchestrator' },
        ]}
        selectedProfileId="default"
        locale="en"
        onSelect={onSelect}
      />
    );

    expect(screen.queryByRole('radio', { name: 'Default' })).toBeNull();
    expect(screen.queryByRole('radio', { name: 'Reasoning' })).toBeNull();
  });
});
