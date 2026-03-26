import { describe, expect, it } from 'vitest';

import { normalizeMemoryDocuments } from '../memoryDebugUtils';

describe('normalizeMemoryDocuments', () => {
  it('reads documents from data.documents', () => {
    const payload = {
      success: true,
      data: {
        documents: [
          { documentId: 'doc-1', namespace: 'conversations', title: 'First' },
          { id: 'doc-2', namespace: 'skills', name: 'Second' },
        ],
      },
    };

    expect(normalizeMemoryDocuments(payload)).toEqual([
      {
        documentId: 'doc-1',
        namespace: 'conversations',
        title: 'First',
        raw: { documentId: 'doc-1', namespace: 'conversations', title: 'First' },
      },
      {
        documentId: 'doc-2',
        namespace: 'skills',
        title: 'Second',
        raw: { id: 'doc-2', namespace: 'skills', name: 'Second' },
      },
    ]);
  });

  it('returns empty for unsupported shapes', () => {
    expect(normalizeMemoryDocuments({ data: { foo: [] } })).toEqual([]);
    expect(normalizeMemoryDocuments(null)).toEqual([]);
  });
});

