import { describe, expect, it } from 'vitest';

import { parseHarnessInitSnapshot } from './harnessInitService';

describe('parseHarnessInitSnapshot', () => {
  it('parses a well-formed snapshot envelope', () => {
    const snap = parseHarnessInitSnapshot({
      snapshot: {
        overall: 'running',
        started_at: '2026-06-23T00:00:00Z',
        finished_at: null,
        steps: [
          {
            id: 'python_runtime',
            label: 'Python runtime',
            required: false,
            state: 'done',
            message: 'already provisioned',
            percent: 100,
            updated_at: '2026-06-23T00:00:01Z',
          },
          {
            id: 'spacy',
            label: 'spaCy',
            required: false,
            state: 'running',
            message: null,
            percent: null,
            updated_at: null,
          },
        ],
      },
    });

    expect(snap).not.toBeNull();
    expect(snap?.overall).toBe('running');
    expect(snap?.startedAt).toBe('2026-06-23T00:00:00Z');
    expect(snap?.steps).toHaveLength(2);
    expect(snap?.steps[0]).toMatchObject({ id: 'python_runtime', state: 'done', percent: 100 });
    expect(snap?.steps[1]).toMatchObject({ id: 'spacy', state: 'running', percent: null });
  });

  it('returns null when the envelope has no snapshot', () => {
    expect(parseHarnessInitSnapshot({})).toBeNull();
    expect(parseHarnessInitSnapshot(null)).toBeNull();
    expect(parseHarnessInitSnapshot({ snapshot: 'nope' })).toBeNull();
  });

  it('returns null on an unknown overall state', () => {
    expect(parseHarnessInitSnapshot({ snapshot: { overall: 'bogus', steps: [] } })).toBeNull();
  });

  it('drops malformed step entries but keeps valid ones', () => {
    const snap = parseHarnessInitSnapshot({
      snapshot: {
        overall: 'done',
        steps: [
          { id: 'node_runtime', state: 'done' },
          { id: 'missing_state' },
          { state: 'done' },
          { id: 'bad_state', state: 'exploded' },
        ],
      },
    });

    expect(snap?.steps).toHaveLength(1);
    expect(snap?.steps[0].id).toBe('node_runtime');
    // Defaults applied for absent optional fields.
    expect(snap?.steps[0].label).toBe('node_runtime');
    expect(snap?.steps[0].required).toBe(false);
  });
});
