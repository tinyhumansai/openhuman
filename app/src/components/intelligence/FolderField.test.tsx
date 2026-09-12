/**
 * Tests for the folder memory-source path field (#5831).
 *
 * The defect these guard against: Browse used to be an
 * `<input type="file" webkitdirectory>`, whose handler stored
 * `webkitRelativePath.split('/')[0]` when the non-standard `File.path` was
 * absent. That is the chosen directory's bare *name*, and no renderer Wry
 * ships (WKWebView, WebView2, WebKitGTK) provides `File.path`, so it was the
 * value stored every time. The source then looked configured and failed once
 * per sync cycle, forever, with `folder does not exist: docs`.
 *
 * The invariant asserted here is therefore not "Browse works" but the
 * stronger "Browse never writes a value that cannot resolve".
 */
import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import { pickDirectoryNatively } from '../../utils/tauriCommands/directoryPicker';
import { KindFields } from './AddMemorySourceFields';

vi.mock('../../utils/tauriCommands/directoryPicker', () => ({ pickDirectoryNatively: vi.fn() }));

const mockPick = vi.mocked(pickDirectoryNatively);

function renderFolderFields(path = '') {
  const setPath = vi.fn();
  renderWithProviders(
    <KindFields
      kind="folder"
      path={path}
      setPath={setPath}
      glob=""
      setGlob={vi.fn()}
      url=""
      setUrl={vi.fn()}
      branch=""
      setBranch={vi.fn()}
      query=""
      setQuery={vi.fn()}
      selector=""
      setSelector={vi.fn()}
      connections={[]}
      loadingConnections={false}
      supportedToolkits={null}
      connectionId=""
      setConnection={vi.fn()}
    />
  );
  return { setPath };
}

const UNAVAILABLE_COPY = 'Could not determine where that folder is. Type its full path instead.';

describe('FolderField (folder memory source)', () => {
  beforeEach(() => {
    mockPick.mockReset();
  });

  it('stores the absolute path the native chooser returned', async () => {
    mockPick.mockResolvedValue({ ok: true, path: '/Users/you/notes' });
    const { setPath } = renderFolderFields();

    fireEvent.click(screen.getByRole('button', { name: /browse/i }));

    await waitFor(() => expect(setPath).toHaveBeenCalledWith('/Users/you/notes'));
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('stores nothing and explains itself when no path can be obtained', async () => {
    mockPick.mockResolvedValue({ ok: false, reason: 'unavailable' });
    const { setPath } = renderFolderFields();

    fireEvent.click(screen.getByRole('button', { name: /browse/i }));

    expect(await screen.findByRole('alert')).toHaveTextContent(UNAVAILABLE_COPY);
    expect(setPath).not.toHaveBeenCalled();
  });

  it('stores nothing when the host chooser tried and failed', async () => {
    mockPick.mockResolvedValue({ ok: false, reason: 'failed', message: 'no portal' });
    const { setPath } = renderFolderFields();

    fireEvent.click(screen.getByRole('button', { name: /browse/i }));

    expect(await screen.findByRole('alert')).toHaveTextContent(UNAVAILABLE_COPY);
    expect(setPath).not.toHaveBeenCalled();
  });

  it('leaves the field untouched and silent when the user cancels', async () => {
    mockPick.mockResolvedValue({ ok: false, reason: 'cancelled' });
    const { setPath } = renderFolderFields();

    fireEvent.click(screen.getByRole('button', { name: /browse/i }));

    await waitFor(() => expect(mockPick).toHaveBeenCalled());
    expect(setPath).not.toHaveBeenCalled();
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('clears a previous error once the user types a path by hand', async () => {
    mockPick.mockResolvedValue({ ok: false, reason: 'unavailable' });
    const { setPath } = renderFolderFields();

    fireEvent.click(screen.getByRole('button', { name: /browse/i }));
    expect(await screen.findByRole('alert')).toBeInTheDocument();

    fireEvent.change(screen.getByPlaceholderText('/Users/you/notes'), {
      target: { value: '/Users/you/docs' },
    });

    expect(screen.queryByRole('alert')).toBeNull();
    expect(setPath).toHaveBeenCalledWith('/Users/you/docs');
  });

  it('no longer renders a directory input that cannot report a path', () => {
    const { container } = renderWithProviders(
      <KindFields
        kind="folder"
        path=""
        setPath={vi.fn()}
        glob=""
        setGlob={vi.fn()}
        url=""
        setUrl={vi.fn()}
        branch=""
        setBranch={vi.fn()}
        query=""
        setQuery={vi.fn()}
        selector=""
        setSelector={vi.fn()}
        connections={[]}
        loadingConnections={false}
        supportedToolkits={null}
        connectionId=""
        setConnection={vi.fn()}
      />
    );

    expect(container.querySelector('input[type="file"]')).toBeNull();
    expect(container.querySelector('[webkitdirectory]')).toBeNull();
  });

  it('gives Browse a stable analytics id rather than a DOM-order fallback', () => {
    mockPick.mockResolvedValue({ ok: false, reason: 'cancelled' });
    renderFolderFields();

    expect(screen.getByRole('button', { name: /browse/i })).toHaveAttribute(
      'data-analytics-id',
      'brain-sources-folder-browse'
    );
  });
});
