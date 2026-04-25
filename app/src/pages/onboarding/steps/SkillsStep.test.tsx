import { fireEvent, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import '../../../test/mockDefaultSkillStatusHooks';
import { renderWithProviders } from '../../../test/test-utils';
import SkillsStep from './SkillsStep';

vi.mock('../components/WebviewLoginModal', () => ({
  default: ({
    label,
    onConnected,
    onClose,
  }: {
    label: string;
    onConnected: (accountId: string) => void;
    onClose: () => void;
  }) => (
    <div role="dialog">
      <p>Sign in to {label}</p>
      <button onClick={() => onConnected('acct-test')}>Mark connected</button>
      <button onClick={onClose}>Cancel</button>
    </div>
  ),
}));

describe('Onboarding SkillsStep', () => {
  it('shows the gmail webview-login card and skips when nothing is connected', async () => {
    const onNext = vi.fn().mockResolvedValue(undefined);
    renderWithProviders(<SkillsStep onNext={onNext} />);

    expect(screen.getByText('Gmail')).toBeInTheDocument();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Skip for Now' }));
    expect(onNext).toHaveBeenCalledWith([]);
  });

  it('opens the webview login modal when the gmail card is clicked', () => {
    renderWithProviders(<SkillsStep onNext={vi.fn()} />);

    fireEvent.click(screen.getByTestId('onboarding-skills-gmail-button'));
    expect(screen.getByRole('dialog')).toBeInTheDocument();
    expect(screen.getByText('Sign in to Gmail')).toBeInTheDocument();
  });

  it('marks gmail connected and forwards webview:gmail on continue', async () => {
    const onNext = vi.fn().mockResolvedValue(undefined);
    renderWithProviders(<SkillsStep onNext={onNext} />);

    fireEvent.click(screen.getByTestId('onboarding-skills-gmail-button'));
    fireEvent.click(screen.getByRole('button', { name: 'Mark connected' }));

    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(screen.getByText('Connected')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));
    expect(onNext).toHaveBeenCalledWith(['webview:gmail']);
  });
});
