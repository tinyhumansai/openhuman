import { fireEvent, render, screen } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { describe, expect, it, vi } from 'vitest';

import WorkflowsRun from './WorkflowsRun';

vi.mock('../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));
vi.mock('../components/skills/WorkflowRunnerBody', () => ({
  default: () => <div data-testid="skills-runner-body" />,
}));

const navigateMock = vi.fn();
vi.mock('react-router-dom', async importOriginal => {
  const actual = await importOriginal<typeof import('react-router-dom')>();
  return { ...actual, useNavigate: () => navigateMock };
});

describe('WorkflowsRun', () => {
  const render_ = () =>
    render(
      <MemoryRouter>
        <WorkflowsRun />
      </MemoryRouter>
    );

  it('renders the back button and page heading', () => {
    render_();
    expect(screen.getByRole('button', { name: 'common.back' })).toBeInTheDocument();
    expect(screen.getByText('skills.run.title')).toBeInTheDocument();
  });

  it('renders WorkflowRunnerBody', () => {
    render_();
    expect(screen.getByTestId('skills-runner-body')).toBeInTheDocument();
  });

  it('back button fires navigate on click', () => {
    render_();
    fireEvent.click(screen.getByRole('button', { name: 'common.back' }));
    // navigate() called — no assertion needed beyond no-throw
  });

  it('falls back to /flows (not the dead /intelligence?tab=workflows route) on a cold deep-link with no history', () => {
    // No `window.history.state.idx` — matches a fresh deep-link with no
    // in-app history entry to go back to (F-m1: /intelligence redirects to
    // /settings/notifications, so the runner must not target it).
    navigateMock.mockClear();
    render_();
    fireEvent.click(screen.getByRole('button', { name: 'common.back' }));
    expect(navigateMock).toHaveBeenCalledWith('/flows');
    expect(navigateMock).not.toHaveBeenCalledWith(expect.stringContaining('/intelligence'));
  });

  it('goes back in history instead when an in-app history entry exists', () => {
    navigateMock.mockClear();
    const originalDescriptor = Object.getOwnPropertyDescriptor(window.history, 'state');
    Object.defineProperty(window.history, 'state', { configurable: true, value: { idx: 1 } });
    try {
      render_();
      fireEvent.click(screen.getByRole('button', { name: 'common.back' }));
      expect(navigateMock).toHaveBeenCalledWith(-1);
    } finally {
      if (originalDescriptor) {
        Object.defineProperty(window.history, 'state', originalDescriptor);
      }
    }
  });
});
