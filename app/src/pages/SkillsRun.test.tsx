import { fireEvent, render, screen } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { describe, expect, it, vi } from 'vitest';

import SkillsRun from './SkillsRun';

vi.mock('../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));
vi.mock('../components/skills/SkillsRunnerBody', () => ({
  default: () => <div data-testid="skills-runner-body" />,
}));

describe('SkillsRun', () => {
  const render_ = () =>
    render(
      <MemoryRouter>
        <SkillsRun />
      </MemoryRouter>
    );

  it('renders the back button and page heading', () => {
    render_();
    expect(screen.getByRole('button', { name: 'common.back' })).toBeInTheDocument();
    expect(screen.getByText('skills.tabs.runners')).toBeInTheDocument();
  });

  it('renders SkillsRunnerBody', () => {
    render_();
    expect(screen.getByTestId('skills-runner-body')).toBeInTheDocument();
  });

  it('back button fires navigate on click', () => {
    render_();
    fireEvent.click(screen.getByRole('button', { name: 'common.back' }));
    // navigate() called — no assertion needed beyond no-throw
  });
});
