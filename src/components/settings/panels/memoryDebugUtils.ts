import type { MemoryDebugDocument } from '../../../utils/tauriCommands';

function asArray(value: unknown): unknown[] {
  return Array.isArray(value) ? value : [];
}

function asRecord(value: unknown): Record<string, unknown> | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return null;
  }
  return value as Record<string, unknown>;
}

function pickFirstString(record: Record<string, unknown>, keys: string[]): string | undefined {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === 'string' && value.trim()) {
      return value;
    }
  }
  return undefined;
}

function findDocumentsArray(payload: unknown): unknown[] {
  if (Array.isArray(payload)) {
    return payload;
  }

  const root = asRecord(payload);
  if (!root) return [];

  const rootCandidates = ['documents', 'items', 'results'];
  for (const key of rootCandidates) {
    const value = root[key];
    if (Array.isArray(value)) return value;
  }

  const data = asRecord(root.data);
  if (!data) return [];

  const dataCandidates = ['documents', 'items', 'results'];
  for (const key of dataCandidates) {
    const value = data[key];
    if (Array.isArray(value)) return value;
  }

  return [];
}

export function normalizeMemoryDocuments(payload: unknown): MemoryDebugDocument[] {
  const items = findDocumentsArray(payload);
  const normalized: MemoryDebugDocument[] = [];

  for (const item of asArray(items)) {
    const record = asRecord(item);
    if (!record) continue;

    const documentId = pickFirstString(record, ['documentId', 'document_id', 'id']);
    const namespace = pickFirstString(record, ['namespace']);
    const title = pickFirstString(record, ['title', 'name']);

    if (!documentId || !namespace) continue;

    normalized.push({ documentId, namespace, title, raw: item });
  }

  return normalized;
}
