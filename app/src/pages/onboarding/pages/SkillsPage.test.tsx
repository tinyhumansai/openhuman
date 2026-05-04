import { fireEvent, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import SkillsPage from './SkillsPage';

const navigateMock = vi.hoisted(() => vi.fn());
const setDraftMock = vi.hoisted(() => vi.fn());
const completeAndExitMock = vi.hoisted(() => vi.fn());

vi.mock('react-router-dom', async importOriginal => {
  const actual = await importOriginal<typeof import('react-router-dom')>();
  return { ...actual, useNavigate: () => navigateMock };
});

vi.mock('../OnboardingContext', () => ({
  useOnboardingContext: () => ({ setDraft: setDraftMock, completeAndExit: completeAndExitMock }),
}));

vi.mock('../steps/SkillsStep', () => ({
  default: (props: { onNext: (payload: { sources: string[] }) => void | Promise<void> }) => (
    <div>
      <button onClick={() => void props.onNext({ sources: ['composio:gmail'] })}>
        Next with composio
      </button>
      <button onClick={() => void props.onNext({ sources: [] })}>Next without composio</button>
    </div>
  ),
}));

describe('SkillsPage', () => {
  it('routes to context when a composio source is selected', async () => {
    renderWithProviders(<SkillsPage />);

    fireEvent.click(screen.getByRole('button', { name: 'Next with composio' }));

    await waitFor(() => {
      expect(navigateMock).toHaveBeenCalledWith('/onboarding/context');
    });
    expect(setDraftMock).toHaveBeenCalledTimes(1);
    const updater = setDraftMock.mock.calls[0][0] as (prev: { connectedSources?: string[] }) => {
      connectedSources: string[];
    };
    expect(updater({ connectedSources: [] }).connectedSources).toEqual(['composio:gmail']);
    expect(completeAndExitMock).not.toHaveBeenCalled();
  });

  it('completes onboarding immediately when no composio source is selected', async () => {
    completeAndExitMock.mockResolvedValue(undefined);
    renderWithProviders(<SkillsPage />);

    fireEvent.click(screen.getByRole('button', { name: 'Next without composio' }));

    await waitFor(() => {
      expect(completeAndExitMock).toHaveBeenCalledTimes(1);
    });
    expect(navigateMock).not.toHaveBeenCalledWith('/onboarding/context');
    expect(setDraftMock).toHaveBeenCalledTimes(1);
    const updater = setDraftMock.mock.calls[0][0] as (prev: { connectedSources?: string[] }) => {
      connectedSources: string[];
    };
    expect(updater({ connectedSources: ['old'] }).connectedSources).toEqual([]);
  });
});
