/**
 * MemoryDebugPanel coverage tests.
 *
 * Target uncovered lines (from diff-cover report):
 * 215,224,234-235,245,256-257,282,288,290-291,309,316,325,338-340,346,348,354,
 * 356,362,364,370,376,382,399,403,407-408,417,427,429
 *
 * These cover:
 * - Documents section: list rendering, doc row (with/without title), delete btn
 * - Namespaces section: list rendering, empty state
 * - Query & Recall: query button, recall button, queryResult/recallResult rendering
 * - Clear Namespace section: select when namespaces exist, text field fallback,
 *   clear success/failure paths
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({ navigateBack: vi.fn(), breadcrumbs: [] }),
}));

vi.mock('../../components/SettingsBackButton', () => ({ default: () => null }));

vi.mock('../../../intelligence/MemoryTextWithEntities', () => ({
  MemoryTextWithEntities: ({ text }: { text: string }) => (
    <span data-testid="mem-text">{text}</span>
  ),
}));

const {
  mockListDocuments,
  mockListNamespaces,
  mockDeleteDocument,
  mockQueryNamespace,
  mockRecallNamespace,
  mockClearNamespace,
} = vi.hoisted(() => ({
  mockListDocuments: vi.fn(),
  mockListNamespaces: vi.fn(),
  mockDeleteDocument: vi.fn(),
  mockQueryNamespace: vi.fn(),
  mockRecallNamespace: vi.fn(),
  mockClearNamespace: vi.fn(),
}));

vi.mock('../../../../utils/tauriCommands', () => ({
  memoryClearNamespace: (...args: unknown[]) => mockClearNamespace(...args),
  memoryDeleteDocument: (...args: unknown[]) => mockDeleteDocument(...args),
  memoryListDocuments: (...args: unknown[]) => mockListDocuments(...args),
  memoryListNamespaces: (...args: unknown[]) => mockListNamespaces(...args),
  memoryQueryNamespace: (...args: unknown[]) => mockQueryNamespace(...args),
  memoryRecallNamespace: (...args: unknown[]) => mockRecallNamespace(...args),
}));

// normalizeMemoryDocuments returns MemoryDebugDocument[] from raw response.
// Mock it to pass through the data we supply.
vi.mock('../memoryDebugUtils', () => ({
  normalizeMemoryDocuments: (raw: unknown) => {
    if (!raw || typeof raw !== 'object') return [];
    const r = raw as { documents?: unknown[] };
    if (!Array.isArray(r.documents)) return [];
    return r.documents;
  },
}));

// ------------------------------------------------------------------
// Factory helpers
// ------------------------------------------------------------------

const makeDoc = (overrides: Record<string, unknown> = {}) => ({
  documentId: 'doc-alpha',
  namespace: 'ns-test',
  title: null,
  ...overrides,
});

const makeRawPayload = (docs: unknown[] = []) => ({ documents: docs });

const queryResult = { text: 'query result text here', entities: [] };

const recallResult = { text: 'recall result text here', entities: [] };

describe('MemoryDebugPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // Default: empty state
    mockListDocuments.mockResolvedValue(makeRawPayload());
    mockListNamespaces.mockResolvedValue([]);
    mockDeleteDocument.mockResolvedValue({});
    mockQueryNamespace.mockResolvedValue(queryResult);
    mockRecallNamespace.mockResolvedValue(recallResult);
    mockClearNamespace.mockResolvedValue({ cleared: true, namespace: 'ns-test' });
  });

  async function renderPanel() {
    const { default: MemoryDebugPanel } = await import('../MemoryDebugPanel');
    return render(<MemoryDebugPanel />);
  }

  // ── Documents section ──────────────────────────────────────────────────────

  it('renders the panel root test-id (smoke test)', async () => {
    await renderPanel();
    expect(screen.getByTestId('memory-debug-panel')).toBeInTheDocument();
  });

  it('renders empty state when no documents (line 234)', async () => {
    mockListDocuments.mockResolvedValue(makeRawPayload([]));
    await renderPanel();
    await waitFor(() => expect(screen.getByText('memory.noDocumentsFound')).toBeInTheDocument());
  });

  it('renders document rows with id and namespace (lines 234-235, 239, 242)', async () => {
    const doc = makeDoc({ documentId: 'doc-beta', namespace: 'ns-alpha', title: null });
    mockListDocuments.mockResolvedValue(makeRawPayload([doc]));
    await renderPanel();

    await waitFor(() => expect(screen.getByText('doc-beta')).toBeInTheDocument());
    expect(screen.getByText('ns-alpha')).toBeInTheDocument();
  });

  it('renders optional title when doc has one (line 245-248)', async () => {
    const doc = makeDoc({
      documentId: 'doc-gamma',
      namespace: 'ns-beta',
      title: 'My document title',
    });
    mockListDocuments.mockResolvedValue(makeRawPayload([doc]));
    await renderPanel();

    await waitFor(() => expect(screen.getByText('My document title')).toBeInTheDocument());
  });

  it('calls memoryDeleteDocument after confirm dialog (lines 256-257, 282)', async () => {
    const doc = makeDoc({ documentId: 'doc-delta', namespace: 'ns-gamma' });
    mockListDocuments.mockResolvedValue(makeRawPayload([doc]));
    // Mock window.confirm to auto-accept
    vi.spyOn(window, 'confirm').mockReturnValue(true);

    await renderPanel();
    await waitFor(() => expect(screen.getByText('doc-delta')).toBeInTheDocument());

    fireEvent.click(screen.getByText('memory.delete'));

    await waitFor(() => expect(mockDeleteDocument).toHaveBeenCalledWith('doc-delta', 'ns-gamma'));
  });

  it('does not delete when confirm is cancelled (line 282)', async () => {
    const doc = makeDoc({ documentId: 'doc-epsilon', namespace: 'ns-delta' });
    mockListDocuments.mockResolvedValue(makeRawPayload([doc]));
    vi.spyOn(window, 'confirm').mockReturnValue(false);

    await renderPanel();
    await waitFor(() => screen.getByText('memory.delete'));
    fireEvent.click(screen.getByText('memory.delete'));

    expect(mockDeleteDocument).not.toHaveBeenCalled();
  });

  // ── Namespaces section ─────────────────────────────────────────────────────

  it('renders empty state when no namespaces (line 299)', async () => {
    mockListNamespaces.mockResolvedValue([]);
    await renderPanel();
    await waitFor(() => expect(screen.getByText('memory.noNamespacesFound')).toBeInTheDocument());
  });

  it('renders namespace chips when namespaces exist (lines 288, 290-291)', async () => {
    mockListNamespaces.mockResolvedValue(['ns-workspace', 'ns-channel']);
    await renderPanel();

    // Namespace chips in the namespaces section (may also appear in Clear Namespace select)
    await waitFor(() =>
      expect(screen.getAllByText('ns-workspace').length).toBeGreaterThanOrEqual(1)
    );
    expect(screen.getAllByText('ns-channel').length).toBeGreaterThanOrEqual(1);
  });

  // ── Query & Recall section ─────────────────────────────────────────────────

  it('calls memoryQueryNamespace and renders result (lines 309, 316, 325, 362, 364)', async () => {
    mockListNamespaces.mockResolvedValue(['ns-workspace']);
    mockQueryNamespace.mockResolvedValue(queryResult);
    await renderPanel();

    // Fill namespace input
    const nsInput = screen.getByPlaceholderText('memory.namespace');
    fireEvent.change(nsInput, { target: { value: 'ns-workspace' } });

    // Fill query text
    const queryTextarea = screen.getByPlaceholderText('memory.queryText');
    fireEvent.change(queryTextarea, { target: { value: 'what did I learn?' } });

    // Click Query button
    fireEvent.click(screen.getByText('memory.query'));

    await waitFor(() =>
      expect(mockQueryNamespace).toHaveBeenCalledWith(
        'ns-workspace',
        'what did I learn?',
        expect.any(Number)
      )
    );

    // Result label and text rendered (lines 362, 364, 370)
    await waitFor(() => expect(screen.getByText('memory.queryResult')).toBeInTheDocument());
    expect(screen.getByText('query result text here')).toBeInTheDocument();
  });

  it('calls memoryRecallNamespace and renders result (lines 338-340, 346, 348, 376, 382)', async () => {
    mockRecallNamespace.mockResolvedValue(recallResult);
    await renderPanel();

    const nsInput = screen.getByPlaceholderText('memory.namespace');
    fireEvent.change(nsInput, { target: { value: 'ns-workspace' } });

    // Query textarea can remain empty for recall
    fireEvent.click(screen.getByText('memory.recall'));

    await waitFor(() =>
      expect(mockRecallNamespace).toHaveBeenCalledWith('ns-workspace', expect.any(Number))
    );

    await waitFor(() => expect(screen.getByText('memory.recallResult')).toBeInTheDocument());
    expect(screen.getByText('recall result text here')).toBeInTheDocument();
  });

  it('shows query error when queryNamespace rejects (lines 354, 356)', async () => {
    mockQueryNamespace.mockRejectedValue(new Error('query failed due to timeout'));
    await renderPanel();

    const nsInput = screen.getByPlaceholderText('memory.namespace');
    fireEvent.change(nsInput, { target: { value: 'ns-test' } });
    const queryTextarea = screen.getByPlaceholderText('memory.queryText');
    fireEvent.change(queryTextarea, { target: { value: 'search term' } });

    fireEvent.click(screen.getByText('memory.query'));

    await waitFor(() =>
      expect(screen.getByText(/query failed due to timeout/)).toBeInTheDocument()
    );
  });

  it('shows recall error when recallNamespace rejects (lines 346, 348)', async () => {
    mockRecallNamespace.mockRejectedValue(new Error('recall timed out'));
    await renderPanel();

    const nsInput = screen.getByPlaceholderText('memory.namespace');
    fireEvent.change(nsInput, { target: { value: 'ns-test' } });

    fireEvent.click(screen.getByText('memory.recall'));

    await waitFor(() => expect(screen.getByText(/recall timed out/)).toBeInTheDocument());
  });

  // ── Clear Namespace section ────────────────────────────────────────────────

  it('renders SettingsSelect when namespaces exist (lines 399, 403)', async () => {
    mockListNamespaces.mockResolvedValue(['ns-workspace', 'ns-channel']);
    await renderPanel();

    await waitFor(() => expect(screen.getByRole('combobox')).toBeInTheDocument());
    // Options should include each namespace (lines 407-408)
    const select = screen.getByRole('combobox') as HTMLSelectElement;
    const optionValues = Array.from(select.options).map(o => o.value);
    expect(optionValues).toContain('ns-workspace');
    expect(optionValues).toContain('ns-channel');
  });

  it('renders text input fallback when no namespaces exist (line 417)', async () => {
    mockListNamespaces.mockResolvedValue([]);
    await renderPanel();

    await waitFor(() => expect(screen.queryByRole('combobox')).not.toBeInTheDocument());
    // The namespace text field has aria-label from t()
    expect(screen.getByPlaceholderText('memory.exampleNamespace')).toBeInTheDocument();
  });

  it('shows success message after clear (line 427)', async () => {
    mockListNamespaces.mockResolvedValue(['ns-workspace']);
    mockClearNamespace.mockResolvedValue({ cleared: true, namespace: 'ns-workspace' });
    vi.spyOn(window, 'confirm').mockReturnValue(true);
    await renderPanel();

    await waitFor(() => expect(screen.getByRole('combobox')).toBeInTheDocument());

    const select = screen.getByRole('combobox') as HTMLSelectElement;
    fireEvent.change(select, { target: { value: 'ns-workspace' } });

    fireEvent.click(screen.getByText('memory.clear'));

    await waitFor(() => expect(mockClearNamespace).toHaveBeenCalledWith('ns-workspace'));
    await waitFor(() =>
      expect(screen.getByText('memory.clearNamespaceSuccess')).toBeInTheDocument()
    );
  });

  it('shows empty/nothing message when cleared=false (line 429)', async () => {
    mockListNamespaces.mockResolvedValue(['ns-empty']);
    mockClearNamespace.mockResolvedValue({ cleared: false, namespace: 'ns-empty' });
    vi.spyOn(window, 'confirm').mockReturnValue(true);
    await renderPanel();

    await waitFor(() => expect(screen.getByRole('combobox')).toBeInTheDocument());

    const select = screen.getByRole('combobox') as HTMLSelectElement;
    fireEvent.change(select, { target: { value: 'ns-empty' } });

    fireEvent.click(screen.getByText('memory.clear'));

    await waitFor(() => expect(screen.getByText('memory.clearNamespaceEmpty')).toBeInTheDocument());
  });

  it('shows error when clearNamespace rejects', async () => {
    mockListNamespaces.mockResolvedValue(['ns-bad']);
    mockClearNamespace.mockRejectedValue(new Error('clear failed'));
    vi.spyOn(window, 'confirm').mockReturnValue(true);
    await renderPanel();

    await waitFor(() => expect(screen.getByRole('combobox')).toBeInTheDocument());

    const select = screen.getByRole('combobox') as HTMLSelectElement;
    fireEvent.change(select, { target: { value: 'ns-bad' } });

    fireEvent.click(screen.getByText('memory.clear'));

    await waitFor(() => expect(screen.getByText(/clear failed/)).toBeInTheDocument());
  });

  it('does not call clearNamespace when confirm is cancelled', async () => {
    mockListNamespaces.mockResolvedValue(['ns-test']);
    vi.spyOn(window, 'confirm').mockReturnValue(false);
    await renderPanel();

    await waitFor(() => expect(screen.getByRole('combobox')).toBeInTheDocument());

    const select = screen.getByRole('combobox') as HTMLSelectElement;
    fireEvent.change(select, { target: { value: 'ns-test' } });

    fireEvent.click(screen.getByText('memory.clear'));

    expect(mockClearNamespace).not.toHaveBeenCalled();
  });

  it('refresh documents button re-loads (line 215, 224)', async () => {
    await renderPanel();
    // Two refresh buttons: documents section + namespaces section
    await waitFor(() =>
      expect(screen.getAllByText('memory.refresh').length).toBeGreaterThanOrEqual(1)
    );

    const refreshBtns = screen.getAllByText('memory.refresh');
    fireEvent.click(refreshBtns[0]);

    await waitFor(() => expect(mockListDocuments).toHaveBeenCalledTimes(2));
  });
});
