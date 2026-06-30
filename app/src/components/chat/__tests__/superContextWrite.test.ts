import { describe, expect, it } from 'vitest';

import { trackSuperContextWrite, whenSuperContextWriteSettled } from '../superContextWrite';

describe('superContextWrite gate', () => {
  it('resolves immediately when nothing is pending', async () => {
    // Completing the await without throwing is the assertion.
    await whenSuperContextWriteSettled();
  });

  it('waits for a tracked write to settle before resolving', async () => {
    let settled = false;
    const write = new Promise(resolve =>
      setTimeout(() => {
        settled = true;
        resolve('ok');
      }, 5)
    );
    trackSuperContextWrite(write);

    await whenSuperContextWriteSettled();
    expect(settled).toBe(true);
  });

  it('still settles when a tracked write rejects', async () => {
    trackSuperContextWrite(Promise.reject(new Error('rpc down')));
    // Must not throw — the send path only needs the write to have settled.
    await whenSuperContextWriteSettled();
  });
});
