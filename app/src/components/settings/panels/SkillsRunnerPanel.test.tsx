import { render, screen } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { describe, expect, it, vi } from 'vitest';

import SkillsRunnerPanel from './SkillsRunnerPanel';

vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));
vi.mock('../../skills/SkillsRunnerBody', () => ({
  default: () => <div data-testid="skills-runner-body" />,
}));
vi.mock('../components/SettingsHeader', () => ({
  default: ({ title }: { title: string }) => <div data-testid="settings-header">{title}</div>,
}));
vi.mock('../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({ navigateBack: vi.fn(), breadcrumbs: [] }),
}));

describe('SkillsRunnerPanel', () => {
  it('renders the settings header and runner body', () => {
    render(
      <MemoryRouter>
        <SkillsRunnerPanel />
      </MemoryRouter>
    );
    expect(screen.getByTestId('settings-header')).toBeInTheDocument();
    expect(screen.getByTestId('skills-runner-body')).toBeInTheDocument();
  });
});
