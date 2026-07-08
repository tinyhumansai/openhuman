/**
 * Unit tests for `erroredNodeIds` (Phase 3c) — mapping graph-level validation
 * error strings back to the node ids they name, for canvas highlighting.
 */
import { describe, expect, it } from 'vitest';

import { erroredNodeIds } from './flowValidation';

describe('erroredNodeIds', () => {
  it('flags a node named in an invalid-config error', () => {
    const flagged = erroredNodeIds(
      ['invalid config for node agent1: missing prompt'],
      ['trigger', 'agent1']
    );
    expect([...flagged]).toEqual(['agent1']);
  });

  it('flags every id listed in a multiple-triggers error', () => {
    const flagged = erroredNodeIds(
      ['workflow has multiple trigger nodes: ["t1", "t2"]'],
      ['t1', 't2', 'a']
    );
    expect(flagged).toEqual(new Set(['t1', 't2']));
  });

  it('flags nothing for a graph-level error that names no node', () => {
    const flagged = erroredNodeIds(['workflow has no trigger node'], ['t', 'a']);
    expect(flagged.size).toBe(0);
  });

  it('does not partial-match a shorter id embedded in a longer hyphenated id', () => {
    // "agent" must NOT be flagged by an error naming "new-agent-0".
    const flagged = erroredNodeIds(
      ['invalid config for node new-agent-0: bad'],
      ['agent', 'new-agent-0']
    );
    expect(flagged).toEqual(new Set(['new-agent-0']));
  });

  it('returns an empty set when there are no errors', () => {
    expect(erroredNodeIds([], ['t', 'a']).size).toBe(0);
  });
});
