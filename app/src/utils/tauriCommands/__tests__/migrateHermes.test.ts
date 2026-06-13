import { describe, expect, it } from 'vitest';

import { openhumanMigrateHermes } from '../core';

describe('openhumanMigrateHermes', () => {
  it('throws when not running in Tauri', async () => {
    await expect(openhumanMigrateHermes()).rejects.toThrow('Not running in Tauri');
  });
});
