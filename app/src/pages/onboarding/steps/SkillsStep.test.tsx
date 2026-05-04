import { fireEvent, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import '../../../test/mockDefaultSkillStatusHooks';
import { renderWithProviders } from '../../../test/test-utils';
import SkillsStep from './SkillsStep';

const useComposioIntegrationsMock = vi.hoisted(() => vi.fn());
vi.mock('../../../lib/composio/hooks', () => ({
  useComposioIntegrations: useComposioIntegrationsMock,
}));

vi.mock('../../../components/composio/ComposioConnectModal', () => ({
  default: ({
    toolkit,
    onClose,
    onChanged,
  }: {
    toolkit: { name: string };
    onClose: () => void;
    onChanged: () => void;
  }) => (
    <div role="dialog">
      <p>Sign in to {toolkit.name}</p>
      <button
        onClick={() => {
          onChanged();
          onClose();
        }}>
        Mark connected
      </button>
      <button onClick={onClose}>Cancel</button>
    </div>
  ),
}));

function setComposioState(opts: { connected?: boolean; error?: string | null }): void {
  const { connected = false, error = null } = opts;
  const map = new Map();
  if (connected) {
    map.set('gmail', { toolkit: 'gmail', status: 'ACTIVE', composioState: 'connected' });
  }
  useComposioIntegrationsMock.mockReturnValue({
    toolkits: ['gmail'],
    connectionByToolkit: map,
    loading: false,
    error,
    refresh: vi.fn(),
  });
}

describe('Onboarding SkillsStep', () => {
  it('shows the Composio gmail card and skips when nothing is connected', async () => {
    setComposioState({});
    const onNext = vi.fn().mockResolvedValue(undefined);
    renderWithProviders(<SkillsStep onNext={onNext} />);

    expect(screen.getByText('Gmail')).toBeInTheDocument();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Skip for Now' }));
    expect(onNext).toHaveBeenCalledWith({ sources: [] });
  });

  it('opens the Composio connect modal when the gmail card is clicked', () => {
    setComposioState({});
    renderWithProviders(<SkillsStep onNext={vi.fn()} />);

    fireEvent.click(screen.getByTestId('onboarding-skills-gmail-button'));
    expect(screen.getByRole('dialog')).toBeInTheDocument();
    expect(screen.getByText('Sign in to Gmail')).toBeInTheDocument();
  });

  it('forwards composio:gmail on continue when gmail is connected', async () => {
    setComposioState({ connected: true });
    const onNext = vi.fn().mockResolvedValue(undefined);
    renderWithProviders(<SkillsStep onNext={onNext} />);

    expect(screen.getByText('Connected')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));
    expect(onNext).toHaveBeenCalledWith({ sources: ['composio:gmail'] });
  });
});
