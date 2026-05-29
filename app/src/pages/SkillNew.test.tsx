/**
 * SkillNew — Phase 6 coverage.
 *
 * Covers:
 *  - renders the form (delegates to CreateSkillForm) and the header
 *    Cancel/Submit buttons.
 *  - cancel button navigates back to /skills.
 *  - on a successful submit (createSkill resolves), the page
 *    navigates to /skills.
 *  - submit button reflects the form's validity (disabled until both
 *    required fields are filled).
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import SkillNew from './SkillNew';

const stableT = (key: string) => key;
vi.mock('../lib/i18n/I18nContext', () => ({ useT: () => ({ t: stableT }) }));

const hoisted = vi.hoisted(() => ({ createSkill: vi.fn() }));

vi.mock('../services/api/skillsApi', () => ({ skillsApi: { createSkill: hoisted.createSkill } }));

const renderPage = () =>
  render(
    <MemoryRouter initialEntries={['/skills/new']}>
      <Routes>
        <Route path="/skills/new" element={<SkillNew />} />
        <Route path="/skills" element={<div data-testid="dashboard-landed">dashboard</div>} />
      </Routes>
    </MemoryRouter>
  );

describe('SkillNew', () => {
  beforeEach(() => {
    hoisted.createSkill.mockReset();
  });

  it('renders the form and the header CTAs', () => {
    renderPage();
    expect(screen.getByTestId('skill-new-cancel')).toBeInTheDocument();
    expect(screen.getByTestId('skill-new-submit')).toBeInTheDocument();
    // CreateSkillForm renders the name + description inputs.
    expect(screen.getByLabelText(/skills.create.name/i)).toBeInTheDocument();
    expect(screen.getByLabelText(/skills.create.description/i)).toBeInTheDocument();
  });

  it('cancel button navigates back to /skills', () => {
    renderPage();
    fireEvent.click(screen.getByTestId('skill-new-cancel'));
    expect(screen.getByTestId('dashboard-landed')).toBeInTheDocument();
  });

  it('submit is disabled until both required fields are filled', () => {
    renderPage();
    const submit = screen.getByTestId('skill-new-submit') as HTMLButtonElement;
    expect(submit).toBeDisabled();

    fireEvent.change(screen.getByLabelText(/skills.create.name/i), {
      target: { value: 'New Skill' },
    });
    expect(submit).toBeDisabled(); // still missing description

    fireEvent.change(screen.getByLabelText(/skills.create.description/i), {
      target: { value: 'Does something neat.' },
    });
    expect(submit).not.toBeDisabled();
  });

  it('navigates to /skills after a successful submit', async () => {
    hoisted.createSkill.mockResolvedValue({
      id: 'new-skill',
      name: 'New Skill',
      scope: 'user',
      legacy: false,
    });
    renderPage();

    fireEvent.change(screen.getByLabelText(/skills.create.name/i), {
      target: { value: 'New Skill' },
    });
    fireEvent.change(screen.getByLabelText(/skills.create.description/i), {
      target: { value: 'Description.' },
    });

    fireEvent.click(screen.getByTestId('skill-new-submit'));
    await waitFor(() => expect(hoisted.createSkill).toHaveBeenCalled());
    await screen.findByTestId('dashboard-landed');
  });
});
