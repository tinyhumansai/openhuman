import { screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import LlmConnectionsPanel from '../LlmConnectionsPanel';

vi.mock('../../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

vi.mock('../AIPanel', () => ({ default: () => <div data-testid="api-keys-panel" /> }));

describe('LlmConnectionsPanel', () => {
  it('renders the AI settings panel', () => {
    renderWithProviders(<LlmConnectionsPanel />, { initialEntries: ['/connections?tab=llm'] });

    expect(screen.getByTestId('api-keys-panel')).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'pages.settings.ai.llm' })).toBeInTheDocument();
    expect(screen.getByText('connections.header.llm')).toBeInTheDocument();
  });

  // The page used to be chip-backed, with `#local-model` / `#agent-chat`
  // selecting a developer diagnostic. Those panels are deleted; a stale deep
  // link must land on the surviving surface rather than render nothing.
  it.each(['#local-model', '#agent-chat', '#unknown'])(
    'ignores the retired %s hash and still renders AI settings',
    hash => {
      renderWithProviders(<LlmConnectionsPanel />, {
        initialEntries: [`/connections?tab=llm${hash}`],
      });

      expect(screen.getByTestId('api-keys-panel')).toBeInTheDocument();
    }
  );

  it('renders provider and routing chips below the page description', () => {
    renderWithProviders(<LlmConnectionsPanel />, { initialEntries: ['/connections?tab=llm'] });

    expect(screen.getByRole('tablist', { name: 'pages.settings.ai.llm' })).toBeInTheDocument();
    expect(screen.getByTestId('ai-tab-providers')).toBeInTheDocument();
    expect(screen.getByTestId('ai-tab-routing')).toBeInTheDocument();
  });
});
