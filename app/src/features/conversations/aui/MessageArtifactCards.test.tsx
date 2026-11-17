import { configureStore } from '@reduxjs/toolkit';
import { render, screen } from '@testing-library/react';
import { Provider } from 'react-redux';
import { describe, expect, it, vi } from 'vitest';

import chatRuntimeReducer, { upsertArtifactReadyForThread } from '../../../store/chatRuntimeSlice';
import { MessageArtifactCards } from './MessageArtifactCards';

const message = vi.hoisted(() => ({ custom: {} as Record<string, unknown> }));

vi.mock('@assistant-ui/react', () => ({
  useAuiState: (select: (s: unknown) => unknown) =>
    select({ message: { metadata: { custom: message.custom } } }),
}));

vi.mock('../../../lib/i18n/I18nContext', () => ({
  useT: () => ({ t: (k: string) => k, locale: 'en' }),
}));

function renderCards(custom: Record<string, unknown>) {
  message.custom = custom;
  const store = configureStore({ reducer: { chatRuntime: chatRuntimeReducer } });
  for (const [artifactId, requestId] of [
    ['a-1', 'req-last'],
    ['a-2', 'req-last'],
    ['a-3', 'req-last'],
  ]) {
    store.dispatch(
      upsertArtifactReadyForThread({
        threadId: 't-1',
        artifactId,
        kind: 'document',
        title: `Doc ${artifactId}`,
        path: `/tmp/${artifactId}.md`,
        sizeBytes: 12,
        requestId,
      })
    );
  }
  render(
    <Provider store={store}>
      <MessageArtifactCards />
    </Provider>
  );
  return screen.queryAllByTestId('artifact-card-adapter').map(card => card.textContent);
}

describe('MessageArtifactCards', () => {
  it('a reply with rows shows exactly its rows, ignoring stale owners', () => {
    // All three carry the owner `req-last` (a stale guess), but the rows say
    // only a-3 belongs to this reply.
    const cards = renderCards({ extraMetadata: { requestId: 'req-last', artifactIds: ['a-3'] } });
    expect(cards).toHaveLength(1);
    expect(cards[0]).toContain('Doc a-3');
  });

  it('a reply without rows yet falls back to the live owner', () => {
    expect(renderCards({ extraMetadata: { requestId: 'req-last' } })).toHaveLength(3);
  });

  it('the running turn uses the live owner', () => {
    expect(renderCards({ requestId: 'req-last', streaming: true })).toHaveLength(3);
  });

  it('a reply of another turn without rows shows nothing', () => {
    expect(renderCards({ extraMetadata: { requestId: 'req-other' } })).toHaveLength(0);
  });
});
