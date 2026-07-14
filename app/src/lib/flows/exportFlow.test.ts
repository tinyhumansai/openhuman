/**
 * exportFlow (Phase 4d) — pure client-side flow export. Covers the file-name
 * slug, the pretty-printed serialization, and the DOM download side effect
 * (anchor click + object-URL lifecycle).
 */
import { afterEach, describe, expect, it, vi } from 'vitest';

import { downloadFlowGraph, exportFileName, serializeFlowGraph } from './exportFlow';

describe('exportFileName', () => {
  it('slugifies the name and appends .flow.json', () => {
    expect(exportFileName('Daily Digest')).toBe('daily-digest.flow.json');
    expect(exportFileName('  Fetch & Parse API!  ')).toBe('fetch-parse-api.flow.json');
  });

  it('falls back to "workflow" for an empty/symbol-only name', () => {
    expect(exportFileName('')).toBe('workflow.flow.json');
    expect(exportFileName('***')).toBe('workflow.flow.json');
  });
});

describe('serializeFlowGraph', () => {
  it('pretty-prints the graph with a trailing newline', () => {
    const out = serializeFlowGraph({ name: 'x', nodes: [], edges: [] });
    expect(out).toBe('{\n  "name": "x",\n  "nodes": [],\n  "edges": []\n}\n');
  });
});

describe('downloadFlowGraph', () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('triggers an anchor download and revokes the object URL', () => {
    const createObjectURL = vi.fn(() => 'blob:mock');
    const revokeObjectURL = vi.fn();
    vi.stubGlobal('URL', { createObjectURL, revokeObjectURL });
    const click = vi.fn();
    const anchor = { href: '', download: '', click } as unknown as HTMLAnchorElement;
    vi.spyOn(document, 'createElement').mockReturnValue(anchor);
    vi.spyOn(document.body, 'appendChild').mockImplementation(node => node);
    vi.spyOn(document.body, 'removeChild').mockImplementation(node => node);

    const ok = downloadFlowGraph('My Flow', { name: 'My Flow', nodes: [], edges: [] });

    expect(ok).toBe(true);
    expect(anchor.download).toBe('my-flow.flow.json');
    expect(click).toHaveBeenCalledTimes(1);
    expect(createObjectURL).toHaveBeenCalledTimes(1);
    expect(revokeObjectURL).toHaveBeenCalledWith('blob:mock');

    vi.unstubAllGlobals();
  });
});
