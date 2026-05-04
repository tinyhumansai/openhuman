import { describe, it } from 'vitest';

import { __metaAssertPressKeyReachesCaptureListener } from '../../../test/commandTestUtils';

describe('commandTestUtils', () => {
  it('pressKey reaches capture-phase listeners', () => {
    __metaAssertPressKeyReachesCaptureListener();
  });
});
