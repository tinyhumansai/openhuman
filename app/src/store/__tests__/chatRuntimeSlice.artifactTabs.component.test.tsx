import { configureStore } from '@reduxjs/toolkit';
import { fireEvent, render, screen } from '@testing-library/react';
import { Provider } from 'react-redux';
import { describe, expect, it, vi } from 'vitest';

import { ArtifactPreviewPanel } from '../../features/conversations/aui/ArtifactPreviewPanel';
import chatRuntimeReducer, {
  openArtifact,
  upsertArtifactReadyForThread,
} from '../chatRuntimeSlice';

vi.mock('../../lib/i18n/I18nContext', () => ({
  useT: () => ({ t: (k: string) => k, locale: 'en' }),
}));

function makeStore() {
  const store = configureStore({ reducer: { chatRuntime: chatRuntimeReducer } });
  for (const [id, title] of [
    ['a-1', 'Doc One'],
    ['a-2', 'Doc Two'],
  ] as const) {
    store.dispatch(
      upsertArtifactReadyForThread({
        threadId: 't-1',
        artifactId: id,
        kind: 'document',
        title,
        path: `/tmp/${id}.md`,
        sizeBytes: 12,
      })
    );
  }
  store.dispatch(openArtifact('a-1'));
  store.dispatch(openArtifact('a-2'));
  return store;
}

describe('ArtifactPreviewPanel — tab switching & close (#1535)', () => {
  it('renders a tab per open artifact and switching updates activeArtifactId', () => {
    const store = makeStore();
    render(
      <Provider store={store}>
        <ArtifactPreviewPanel />
      </Provider>
    );

    // Two tabs rendered (showTabs = length > 1)
    const tab1 = screen.getByRole('tab', { name: 'Doc One' });
    const tab2 = screen.getByRole('tab', { name: 'Doc Two' });
    expect(tab1).toBeTruthy();
    expect(tab2).toBeTruthy();

    // activeArtifactId starts at a-2 (last opened)
    expect(store.getState().chatRuntime.activeArtifactId).toBe('a-2');

    // Clicking the first tab activates it
    fireEvent.click(tab1);
    expect(store.getState().chatRuntime.activeArtifactId).toBe('a-1');
    // tab list unchanged
    expect(store.getState().chatRuntime.openArtifactIds).toEqual(['a-1', 'a-2']);

    // Clicking the second tab switches back
    fireEvent.click(tab2);
    expect(store.getState().chatRuntime.activeArtifactId).toBe('a-2');
  });

  it('the per-tab X closes that tab and the panel stays on the other tab', () => {
    const store = makeStore();
    render(
      <Provider store={store}>
        <ArtifactPreviewPanel />
      </Provider>
    );

    // Two tabs, each with a close (X) button.
    const closeBtns = screen.getAllByRole('button', { name: 'chat.artifact.tab.close' });
    expect(closeBtns).toHaveLength(2);

    // Close the non-active tab (a-1): it is removed and a-2 stays active.
    fireEvent.click(closeBtns[0]);
    expect(store.getState().chatRuntime.openArtifactIds).toEqual(['a-2']);
    expect(store.getState().chatRuntime.activeArtifactId).toBe('a-2');
  });

  it('the header X closes the whole panel, whatever tabs are open', () => {
    const store = makeStore();
    render(
      <Provider store={store}>
        <ArtifactPreviewPanel />
      </Provider>
    );

    fireEvent.click(screen.getByRole('button', { name: 'chat.artifact.panel.close' }));
    expect(store.getState().chatRuntime.openArtifactIds).toEqual([]);
    expect(store.getState().chatRuntime.activeArtifactId).toBeNull();
  });
});
