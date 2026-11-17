import { configureStore } from '@reduxjs/toolkit';
import { fireEvent, render, screen } from '@testing-library/react';
import { Provider } from 'react-redux';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc } from '../../../services/coreRpcClient';
import chatRuntimeReducer, {
  openArtifact,
  upsertArtifactReadyForThread,
} from '../../../store/chatRuntimeSlice';
import { openUrl } from '../../../utils/openUrl';
import { ArtifactPreviewPanel, fileExtension } from './ArtifactPreviewPanel';

vi.mock('../../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));
vi.mock('../../../utils/openUrl', () => ({ openUrl: vi.fn().mockResolvedValue(undefined) }));

vi.mock('../../../lib/i18n/I18nContext', () => ({
  useT: () => ({ t: (k: string) => k, locale: 'en' }),
}));

const MD = '# Hello\n\nThis is **bold** text.\n';

function renderPanel(kind: string, body: string, failWith?: string, converted = false) {
  // Stub the body fetch BEFORE mounting so the panel's effect resolves it.
  if (failWith) vi.mocked(callCoreRpc).mockRejectedValue(failWith);
  else
    vi.mocked(callCoreRpc).mockResolvedValue({
      kind,
      title: 'Notes',
      version: 1,
      body,
      is_text: true,
      converted,
    });
  const store = configureStore({ reducer: { chatRuntime: chatRuntimeReducer } });
  store.dispatch(
    upsertArtifactReadyForThread({
      threadId: 't-1',
      artifactId: 'a-1',
      kind: 'document',
      title: 'Notes',
      path: '/tmp/n.md',
      sizeBytes: body.length,
    })
  );
  store.dispatch(openArtifact('a-1'));
  render(
    <Provider store={store}>
      <ArtifactPreviewPanel />
    </Provider>
  );
}

describe('ArtifactPreviewPanel — preview rendering by kind (#1535)', () => {
  beforeEach(() => {
    vi.mocked(callCoreRpc).mockReset();
  });

  it('renders markdown as formatted content, not raw source', async () => {
    renderPanel('markdown', MD);

    const h1 = await screen.findByRole('heading', { level: 1 });
    expect(h1.textContent).toBe('Hello');

    // **bold** becomes <strong>, not literal asterisks
    expect(screen.getByText('bold').tagName).toBe('STRONG');

    // The raw markdown source must NOT be shown verbatim.
    expect(screen.queryByText(/# Hello/)).toBeNull();
  });

  it('reads the body from the core', async () => {
    renderPanel('markdown', MD);
    await screen.findByRole('heading', { level: 1 });
    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.ai_read_artifact_body',
      params: { artifact_id: 'a-1' },
    });
  });

  it('renders a converted document as Markdown, read-only', async () => {
    renderPanel('file', '# Quarterly report\n\nRevenue **up**.\n', undefined, true);

    const h1 = await screen.findByRole('heading', { level: 1 });
    expect(h1.textContent).toBe('Quarterly report');
    fireEvent.mouseDown(screen.getByRole('tab', { name: /chat.artifact.view.code/ }));
    // The artifact is the file, not this text: it cannot be edited here.
    expect((screen.getByRole('textbox') as HTMLTextAreaElement).readOnly).toBe(true);
  });

  it('opens an external link in the browser, never in the app window', async () => {
    renderPanel(
      'markdown',
      '[site](https://example.com/page) and [bad](javascript:alert(1)) and [rel](/settings)\n'
    );
    const site = await screen.findByText('site');
    const click = (el: HTMLElement) =>
      fireEvent.click(el, { bubbles: true, cancelable: true }) === false;

    // The default navigation is always cancelled.
    expect(click(site)).toBe(true);
    expect(openUrl).toHaveBeenCalledWith('https://example.com/page');

    vi.mocked(openUrl).mockClear();
    expect(click(screen.getByText('rel'))).toBe(true);
    expect(click(screen.getByText('bad'))).toBe(true);
    expect(openUrl).not.toHaveBeenCalled();
  });

  it('renders GFM constructs (tables) via remark-gfm', async () => {
    renderPanel('markdown', '| a | b |\n| --- | --- |\n| one | two |\n');

    const table = await screen.findByRole('table');
    expect(table).toBeTruthy();
    expect(screen.getByText('one')).toBeTruthy();
  });

  it('keeps rendering HTML kind in the sandboxed iframe (unchanged)', async () => {
    renderPanel('html', '<h1>Hi</h1><p>body</p>');

    const frame = await screen.findByTitle('chat.artifact.htmlPreview');
    expect(frame.tagName).toBe('IFRAME');
    expect(frame.getAttribute('sandbox')).toBe('allow-scripts');
    expect(frame.getAttribute('srcdoc')).toContain('<h1>Hi</h1>');
  });

  it('keeps the editor read-only when the body failed to load', async () => {
    renderPanel('markdown', '', 'artifact unreadable');

    expect(await screen.findByText('artifact unreadable')).toBeTruthy();
    fireEvent.mouseDown(screen.getByRole('tab', { name: /chat.artifact.view.code/ }));
    // Typing here would save the empty text over the artifact.
    expect((screen.getByRole('textbox') as HTMLTextAreaElement).readOnly).toBe(true);
  });

  it('still renders plain text kind as raw preformatted source', async () => {
    renderPanel('text', 'plain **not** markdown\n');

    const pre = await screen.findByText(/plain \*\*not\*\* markdown/);
    expect(pre.tagName).toBe('PRE');
    expect(screen.queryByRole('heading')).toBeNull();
  });
});

describe('fileExtension', () => {
  it('reads the extension from the file name only', () => {
    expect(fileExtension('/files/release-v1.2.md')).toBe('md');
    expect(fileExtension('/files.d/notes')).toBe('');
    expect(fileExtension('C:\\Users\\me\\Files\\deck.pptx')).toBe('pptx');
    expect(fileExtension(undefined)).toBe('');
  });
});
