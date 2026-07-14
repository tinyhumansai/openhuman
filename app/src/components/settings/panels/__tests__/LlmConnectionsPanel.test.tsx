import { fireEvent, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import LlmConnectionsPanel from '../LlmConnectionsPanel';

vi.mock('../../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

vi.mock('../AIPanel', () => ({ default: () => <div data-testid="api-keys-panel" /> }));
vi.mock('../LocalModelDebugPanel', () => ({
  default: () => <div data-testid="local-model-panel" />,
}));
vi.mock('../AgentChatPanel', () => ({ default: () => <div data-testid="agent-chat-panel" /> }));

describe('LlmConnectionsPanel', () => {
  it.each([
    ['#local-model', 'local-model-panel'],
    ['#agent-chat', 'agent-chat-panel'],
  ])('selects the legacy diagnostic surface from %s', (hash, testId) => {
    renderWithProviders(<LlmConnectionsPanel />, {
      initialEntries: [`/connections?tab=llm${hash}`],
    });

    expect(screen.getByTestId(testId)).toBeInTheDocument();
  });

  it('defaults unknown hashes to API keys', () => {
    renderWithProviders(<LlmConnectionsPanel />, {
      initialEntries: ['/connections?tab=llm#unknown'],
    });

    expect(screen.getByTestId('api-keys-panel')).toBeInTheDocument();
  });

  it('updates the rendered panel when a chip is selected', () => {
    renderWithProviders(<LlmConnectionsPanel />, { initialEntries: ['/connections?tab=llm'] });

    fireEvent.click(screen.getByTestId('llm-chip-agent-chat'));
    expect(screen.getByTestId('agent-chat-panel')).toBeInTheDocument();
  });
});
