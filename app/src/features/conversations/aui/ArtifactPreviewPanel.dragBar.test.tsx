import { configureStore } from '@reduxjs/toolkit';
import { render } from '@testing-library/react';
import { Provider } from 'react-redux';
import { describe, expect, it, vi } from 'vitest';

import chatRuntimeReducer, {
  openArtifact,
  upsertArtifactReadyForThread,
} from '../../../store/chatRuntimeSlice';
import { ArtifactPreviewPanel } from './ArtifactPreviewPanel';

const dragBar = vi.hoisted(() => ({ shown: false }));

vi.mock('../../../components/layout/shell/WindowDragBar', () => ({
  WINDOW_DRAG_BAR_HEIGHT: 32,
  hasWindowDragBar: () => dragBar.shown,
}));

vi.mock('../../../lib/i18n/I18nContext', () => ({
  useT: () => ({ t: (k: string) => k, locale: 'en' }),
}));

function renderPanel() {
  const store = configureStore({ reducer: { chatRuntime: chatRuntimeReducer } });
  store.dispatch(
    upsertArtifactReadyForThread({
      threadId: 't-1',
      artifactId: 'a-1',
      kind: 'document',
      title: 'Doc One',
      path: '/tmp/a-1.md',
      sizeBytes: 12,
    })
  );
  store.dispatch(openArtifact('a-1'));
  const { container } = render(
    <Provider store={store}>
      <ArtifactPreviewPanel />
    </Provider>
  );
  return container.firstElementChild as HTMLElement;
}

describe('ArtifactPreviewPanel under the window-drag band', () => {
  it('pushes the header below the band when the band is drawn', () => {
    dragBar.shown = true;
    expect(renderPanel().style.paddingTop).toBe('32px');
  });

  it('adds no padding where there is no band', () => {
    dragBar.shown = false;
    expect(renderPanel().style.paddingTop).toBe('');
  });
});
