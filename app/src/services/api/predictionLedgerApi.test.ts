import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { PredictionRecord } from '../../lib/memory/predictionLedger';
import { LEDGER_PATH, loadLedger, saveLedger } from './predictionLedgerApi';

const mockReadMemoryFile = vi.fn();
const mockWriteMemoryFile = vi.fn();

vi.mock('../../utils/tauriCommands/memory', () => ({
  aiReadMemoryFile: (...args: unknown[]) => mockReadMemoryFile(...args),
  aiWriteMemoryFile: (...args: unknown[]) => mockWriteMemoryFile(...args),
}));

const NOW = 1_700_000_000;

function record(overrides: Partial<PredictionRecord> = {}): PredictionRecord {
  return {
    id: 'p1',
    statement: 'It ships Friday',
    confidence: 0.8,
    createdAt: NOW,
    resolveBy: null,
    outcome: null,
    resolvedAt: null,
    category: null,
    notes: null,
    ...overrides,
  };
}

describe('predictionLedgerApi.loadLedger', () => {
  beforeEach(() => {
    mockReadMemoryFile.mockReset();
  });

  it('returns [] when the read RPC rejects (first run, missing file)', async () => {
    mockReadMemoryFile.mockRejectedValueOnce(new Error('memory file not found'));
    expect(await loadLedger()).toEqual([]);
    expect(mockReadMemoryFile).toHaveBeenCalledWith(LEDGER_PATH);
  });

  it('returns [] for empty/whitespace content', async () => {
    mockReadMemoryFile.mockResolvedValueOnce('   ');
    expect(await loadLedger()).toEqual([]);
  });

  it('throws on a malformed (corrupt) blob rather than silently returning []', async () => {
    mockReadMemoryFile.mockResolvedValueOnce('{not valid json');
    await expect(loadLedger()).rejects.toThrow(/corrupted/);
  });

  it('parses a valid ledger blob', async () => {
    const records = [record({ id: 'a', outcome: 'true', resolvedAt: NOW })];
    mockReadMemoryFile.mockResolvedValueOnce(JSON.stringify(records));
    const out = await loadLedger();
    expect(out).toHaveLength(1);
    expect(out[0].id).toBe('a');
    expect(out[0].outcome).toBe('true');
  });

  it('drops malformed entries inside an otherwise-valid blob', async () => {
    mockReadMemoryFile.mockResolvedValueOnce(
      JSON.stringify([record({ id: 'a' }), { id: 'b', confidence: 0.5 }])
    );
    const out = await loadLedger();
    expect(out.map(r => r.id)).toEqual(['a']); // 'b' missing statement/createdAt
  });
});

describe('predictionLedgerApi.saveLedger', () => {
  beforeEach(() => {
    mockWriteMemoryFile.mockReset();
  });

  it('writes the ledger as pretty-printed JSON to the fixed path', async () => {
    mockWriteMemoryFile.mockResolvedValueOnce(undefined);
    const records = [record({ id: 'a' })];
    await saveLedger(records);
    expect(mockWriteMemoryFile).toHaveBeenCalledWith(LEDGER_PATH, JSON.stringify(records, null, 2));
  });

  it('propagates write errors', async () => {
    mockWriteMemoryFile.mockRejectedValueOnce(new Error('disk full'));
    await expect(saveLedger([record()])).rejects.toThrow('disk full');
  });
});
