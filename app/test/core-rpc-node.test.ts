import { describe, expect, it } from 'vitest';

import { formatRpcCallFailure } from './e2e/helpers/core-rpc-node';

describe('formatRpcCallFailure', () => {
  it('includes the RPC method, status, and error text', () => {
    expect(
      formatRpcCallFailure('openhuman.composio_list_triggers', {
        ok: false,
        httpStatus: 500,
        error: 'Backend returned 500: trigger store unavailable',
      })
    ).toContain(
      'openhuman.composio_list_triggers failed: httpStatus=500 error=Backend returned 500: trigger store unavailable'
    );
  });
});
