import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  PERSONA_FILE_SOUL,
  readPersonaFile,
  resetPersonaFile,
  type WorkspaceFile,
  writePersonaFile,
} from './personaFilesApi';

const mockCallCoreRpc = vi.fn();

vi.mock('../coreRpcClient', () => ({
  callCoreRpc: (...args: unknown[]) => mockCallCoreRpc(...args),
}));

const file: WorkspaceFile = {
  filename: PERSONA_FILE_SOUL,
  contents: 'You are helpful.',
  is_default: false,
};

describe('personaFilesApi', () => {
  beforeEach(() => {
    mockCallCoreRpc.mockReset();
  });

  it('readPersonaFile calls the read RPC with the filename and returns the file', async () => {
    mockCallCoreRpc.mockResolvedValueOnce(file);
    const result = await readPersonaFile('SOUL.md');
    expect(result).toEqual(file);
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.workspace_file_read',
      params: { filename: 'SOUL.md' },
    });
  });

  it('writePersonaFile passes filename + contents to the write RPC', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ ...file, contents: 'new' });
    const result = await writePersonaFile('SOUL.md', 'new');
    expect(result.contents).toBe('new');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.workspace_file_write',
      params: { filename: 'SOUL.md', contents: 'new' },
    });
  });

  it('resetPersonaFile calls the reset RPC with the filename', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ ...file, is_default: true });
    const result = await resetPersonaFile('SOUL.md');
    expect(result.is_default).toBe(true);
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.workspace_file_reset',
      params: { filename: 'SOUL.md' },
    });
  });

  it('propagates RPC errors from each method', async () => {
    mockCallCoreRpc.mockRejectedValue(new Error('boom'));
    await expect(readPersonaFile('SOUL.md')).rejects.toThrow('boom');
    await expect(writePersonaFile('SOUL.md', 'x')).rejects.toThrow('boom');
    await expect(resetPersonaFile('SOUL.md')).rejects.toThrow('boom');
  });

  it('handles non-Error rejections without throwing in the logger', async () => {
    mockCallCoreRpc.mockRejectedValue('stringy failure');
    await expect(readPersonaFile('SOUL.md')).rejects.toBe('stringy failure');
  });
});
