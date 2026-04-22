import { fireEvent, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import '../../../test/mockDefaultSkillStatusHooks';
import { renderWithProviders } from '../../../test/test-utils';
import SkillsStep from './SkillsStep';

const refreshComposio = vi.fn();
let composioToolkits: string[] = [];
let composioConnections = new Map();
let composioLoading = false;
let composioError: string | null = null;

vi.mock('../../../lib/composio/hooks', () => ({
  useComposioIntegrations: () => ({
    toolkits: composioToolkits,
    connectionByToolkit: composioConnections,
    loading: composioLoading,
    error: composioError,
    refresh: refreshComposio,
  }),
}));

vi.mock('../../../components/composio/ComposioConnectModal', () => ({
  default: ({ toolkit, onClose }: { toolkit: { name: string }; onClose: () => void }) => (
    <div>
      <h2>Connect {toolkit.name}</h2>
      <button onClick={onClose}>Close</button>
    </div>
  ),
}));

describe('Onboarding SkillsStep', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    composioToolkits = [];
    composioConnections = new Map();
    composioLoading = false;
    composioError = null;
  });

  it('falls back to the curated composio catalog when the backend allowlist is empty', () => {
    const onNext = vi.fn();
    renderWithProviders(<SkillsStep onNext={onNext} />);

    expect(screen.getByText('Gmail')).toBeInTheDocument();
    expect(screen.getByText('Google Calendar')).toBeInTheDocument();
    expect(screen.getByText('Google Drive')).toBeInTheDocument();
    expect(screen.getByText('Notion')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Skip for Now' })).toBeInTheDocument();
  });

  it('only surfaces the onboarding subset when the backend returns a larger toolkit allowlist', () => {
    composioToolkits = ['gmail', 'github', 'slack', 'notion'];

    renderWithProviders(<SkillsStep onNext={vi.fn()} />);

    expect(screen.getByText('Gmail')).toBeInTheDocument();
    expect(screen.getByText('Notion')).toBeInTheDocument();
    expect(screen.queryByText('GitHub')).not.toBeInTheDocument();
    expect(screen.queryByText('Slack')).not.toBeInTheDocument();
  });

  it('passes connected composio sources through on continue', async () => {
    composioToolkits = ['gmail', 'notion'];
    composioConnections = new Map([
      ['gmail', { id: 'conn_gmail', toolkit: 'gmail', status: 'ACTIVE' }],
      ['notion', { id: 'conn_notion', toolkit: 'notion', status: 'ACTIVE' }],
    ]);
    const onNext = vi.fn().mockResolvedValue(undefined);

    renderWithProviders(<SkillsStep onNext={onNext} />);

    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));

    expect(onNext).toHaveBeenCalledWith(['composio:gmail', 'composio:notion']);
  });
});
