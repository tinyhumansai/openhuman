import { screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import { MemoryWorkspace } from '../MemoryWorkspace';

// Mock useIntelligenceStats — the hook used by MemoryWorkspace
vi.mock('../../../hooks/useIntelligenceStats', () => ({
  useIntelligenceStats: () => ({
    sessions: { total: 5, totalTokens: 1200 },
    memoryFiles: 3,
    entities: { contact: 2, message: 10 },
    isLoading: false,
    refetch: vi.fn(),
  }),
}));

// Override the global tauriCommands mock from setup.ts with memory-specific stubs
vi.mock('../../../utils/tauriCommands', () => ({
  isTauri: vi.fn(() => true),
  memoryListDocuments: vi.fn().mockResolvedValue({
    documents: [
      { documentId: 'doc-1', namespace: 'research', title: 'Paper A' },
      { documentId: 'doc-2', namespace: 'research', title: 'Paper B' },
    ],
  }),
  memoryListNamespaces: vi.fn().mockResolvedValue(['research', 'conversations']),
  aiListMemoryFiles: vi.fn().mockResolvedValue(['2026-03-31.md']),
  aiReadMemoryFile: vi.fn().mockResolvedValue('# Memory\nSome content'),
  aiWriteMemoryFile: vi.fn().mockResolvedValue(undefined),
  memoryDeleteDocument: vi.fn().mockResolvedValue(undefined),
  memoryQueryNamespace: vi.fn().mockResolvedValue('query result'),
  memoryRecallNamespace: vi.fn().mockResolvedValue('recall result'),
  memoryGraphQuery: vi.fn().mockResolvedValue([
    {
      namespace: 'research',
      subject: 'Alice',
      predicate: 'AUTHORED',
      object: 'Paper A',
      attrs: { entity_types: { subject: 'person', object: 'document' } },
      updatedAt: 1700000000,
      evidenceCount: 3,
      orderIndex: null,
      documentIds: ['doc-1'],
      chunkIds: ['doc-1#chunk-1'],
    },
    {
      namespace: 'research',
      subject: 'Bob',
      predicate: 'REVIEWED',
      object: 'Paper A',
      attrs: { entity_types: { subject: 'person', object: 'document' } },
      updatedAt: 1700000001,
      evidenceCount: 1,
      orderIndex: null,
      documentIds: ['doc-1'],
      chunkIds: [],
    },
  ]),
}));

describe('MemoryWorkspace', () => {
  const onToast = vi.fn();

  it('renders the Memory heading', async () => {
    renderWithProviders(<MemoryWorkspace onToast={onToast} />);
    expect(screen.getByText('Memory')).toBeInTheDocument();
  });

  it('displays graph relations after loading', async () => {
    renderWithProviders(<MemoryWorkspace onToast={onToast} />);

    await waitFor(() => {
      expect(screen.getByText('Alice', { selector: 'span' })).toBeInTheDocument();
    });

    expect(screen.getByText('AUTHORED', { selector: 'span' })).toBeInTheDocument();
    expect(screen.getByText('Bob', { selector: 'span' })).toBeInTheDocument();
    expect(screen.getByText('REVIEWED', { selector: 'span' })).toBeInTheDocument();
    // "Paper A" appears in both graph relations and documents list,
    // so just verify at least one instance is present
    expect(screen.getAllByText('Paper A').length).toBeGreaterThanOrEqual(1);
  });

  it('shows evidence count badge when > 1', async () => {
    renderWithProviders(<MemoryWorkspace onToast={onToast} />);

    await waitFor(() => {
      expect(screen.getByText('x3')).toBeInTheDocument();
    });

    // Bob's relation has evidenceCount 1 — should NOT show a badge
    expect(screen.queryByText('x1')).not.toBeInTheDocument();
  });

  it('shows Relations stat in the stats bar', async () => {
    renderWithProviders(<MemoryWorkspace onToast={onToast} />);

    // The stats bar has a "Relations" label
    await waitFor(() => {
      expect(screen.getByText('Relations')).toBeInTheDocument();
    });
  });

  it('renders the Memory Graph section', async () => {
    renderWithProviders(<MemoryWorkspace onToast={onToast} />);

    await waitFor(() => {
      expect(screen.getByText('Memory Graph')).toBeInTheDocument();
    });
  });
});

describe('MemoryWorkspace – no graph relations', () => {
  const onToast = vi.fn();

  it('shows empty-state message when no relations exist', async () => {
    // Override only memoryGraphQuery to return empty
    const tauriMod = await import('../../../utils/tauriCommands');
    vi.mocked(tauriMod.memoryGraphQuery).mockResolvedValueOnce([]);

    renderWithProviders(<MemoryWorkspace onToast={onToast} />);

    await waitFor(() => {
      expect(screen.getByText('No memory graph data yet')).toBeInTheDocument();
    });
  });
});

describe('MemoryWorkspace – non-Tauri environment', () => {
  const onToast = vi.fn();

  it('shows Tauri-required warning when not running in Tauri', async () => {
    const tauriMod = await import('../../../utils/tauriCommands');
    vi.mocked(tauriMod.isTauri).mockReturnValue(false);

    renderWithProviders(<MemoryWorkspace onToast={onToast} />);

    await waitFor(() => {
      expect(
        screen.getByText('Memory workspace requires the desktop Tauri runtime to load real data.')
      ).toBeInTheDocument();
    });

    // Restore for other tests
    vi.mocked(tauriMod.isTauri).mockReturnValue(true);
  });
});
