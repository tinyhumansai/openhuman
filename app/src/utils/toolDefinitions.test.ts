import { describe, expect, it } from 'vitest';

import {
  getDefaultEnabledTools,
  getEnabledRustToolNames,
  normalizeEnabledToolList,
  TOOL_CATALOG,
} from './toolDefinitions';

describe('normalizeEnabledToolList', () => {
  it('converts a Rust tool name to its UI toggle ID', () => {
    // "web_search_tool" is the Rust name for the "web_search" UI toggle.
    // This was the root cause of #2742: handleSave persisted Rust names,
    // the read path checked UI IDs, and the mismatch caused the toggle to
    // always read back as OFF.
    expect(normalizeEnabledToolList(['web_search_tool'])).toEqual(['web_search']);
  });

  it('passes through an entry that is already a UI toggle ID', () => {
    expect(normalizeEnabledToolList(['shell'])).toEqual(['shell']);
  });

  it('handles a mixed list of UI IDs and Rust names', () => {
    const result = normalizeEnabledToolList(['shell', 'web_search_tool', 'file_read']);
    expect(result).toContain('shell');
    expect(result).toContain('web_search');
    expect(result).toContain('file_read');
    expect(result).toHaveLength(3);
  });

  it('deduplicates when multiple Rust names map to the same UI toggle ID', () => {
    // "cron_add", "cron_list", and "cron_remove" all belong to the "cron" toggle.
    const result = normalizeEnabledToolList(['cron_add', 'cron_list', 'cron_remove']);
    expect(result).toEqual(['cron']);
  });

  it('drops unknown entries', () => {
    const result = normalizeEnabledToolList(['shell', 'totally_unknown_tool']);
    expect(result).toEqual(['shell']);
  });

  it('returns empty array for empty input', () => {
    expect(normalizeEnabledToolList([])).toEqual([]);
  });

  it('round-trips with getEnabledRustToolNames for all default enabled tools', () => {
    const defaults = getDefaultEnabledTools();
    const rustNames = getEnabledRustToolNames(defaults);
    const normalized = normalizeEnabledToolList(rustNames);
    // Every default UI ID must survive the round-trip.
    for (const id of defaults) {
      expect(normalized).toContain(id);
    }
  });

  it('normalises a realistic persisted list that would cause the #2742 revert', () => {
    // Simulate what handleSave writes: Rust names for all tools that were ON.
    const persistedRustNames = getEnabledRustToolNames([
      'shell',
      'web_search',
      'http_request',
      'cron',
    ]);
    const normalized = normalizeEnabledToolList(persistedRustNames);
    expect(normalized).toContain('shell');
    expect(normalized).toContain('web_search');
    expect(normalized).toContain('http_request');
    expect(normalized).toContain('cron');
  });

  it('covers every entry in TOOL_CATALOG — all rustToolNames reverse-map correctly', () => {
    for (const tool of TOOL_CATALOG) {
      for (const rustName of tool.rustToolNames) {
        const result = normalizeEnabledToolList([rustName]);
        expect(result).toEqual([tool.id]);
      }
    }
  });
});
