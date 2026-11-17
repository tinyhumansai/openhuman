import { screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { createTestStore, renderWithProviders } from '../../../test/test-utils';
import { InterruptedTurnNotice } from './InterruptedTurnNotice';

vi.mock('../../../lib/i18n/I18nContext', async importOriginal => ({
  ...(await importOriginal<object>()),
  useT: () => ({ t: (key: string) => key }),
}));

function renderFor(
  selectedThreadId: string | null,
  interrupted: Record<string, { requestId: string; content: string; thinking: string }>
) {
  const base = createTestStore().getState() as unknown as {
    thread: Record<string, unknown>;
    chatRuntime: Record<string, unknown>;
  };
  const store = createTestStore({
    thread: { ...base.thread, selectedThreadId },
    chatRuntime: { ...base.chatRuntime, interruptedAssistantByThread: interrupted },
  });
  return renderWithProviders(<InterruptedTurnNotice />, { store });
}

describe('InterruptedTurnNotice', () => {
  it('shows the partial reply and the interrupted marker for the open thread', () => {
    renderFor('t1', { t1: { requestId: 'r1', content: 'Half an answer', thinking: '' } });
    expect(screen.getByTestId('interrupted-turn')).toHaveTextContent('Half an answer');
    expect(screen.getByText('chat.message.interrupted')).toBeInTheDocument();
  });

  it('shows only the marker when nothing streamed', () => {
    renderFor('t1', { t1: { requestId: 'r1', content: '   ', thinking: 'hmm' } });
    expect(screen.getByTestId('interrupted-turn')).toHaveTextContent('chat.message.interrupted');
  });

  it('renders nothing for another thread or with no thread open', () => {
    const { unmount } = renderFor('t2', { t1: { requestId: 'r1', content: 'x', thinking: '' } });
    expect(screen.queryByTestId('interrupted-turn')).toBeNull();
    unmount();
    renderFor(null, { t1: { requestId: 'r1', content: 'x', thinking: '' } });
    expect(screen.queryByTestId('interrupted-turn')).toBeNull();
  });
});
