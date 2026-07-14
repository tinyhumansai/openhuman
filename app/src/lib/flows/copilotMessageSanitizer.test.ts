import { describe, expect, it } from 'vitest';

import { unwrapToolCallEnvelope } from './copilotMessageSanitizer';

describe('unwrapToolCallEnvelope', () => {
  it('returns plain text unchanged with no tool names', () => {
    const result = unwrapToolCallEnvelope('Hello, how can I help?');
    expect(result).toEqual({ text: 'Hello, how can I help?', toolNames: [] });
  });

  it('extracts the content text and tool name from a valid native envelope', () => {
    const raw = JSON.stringify({
      content: 'Let me build that for you.',
      tool_calls: [{ id: 'call_1', name: 'propose_workflow', arguments: '{}' }],
    });
    const result = unwrapToolCallEnvelope(raw);
    expect(result).toEqual({ text: 'Let me build that for you.', toolNames: ['propose_workflow'] });
  });

  it('passes through JSON with extra/unexpected top-level keys (not an envelope)', () => {
    const raw = JSON.stringify({ content: 'hi', tool_calls: [], unexpected: true });
    const result = unwrapToolCallEnvelope(raw);
    expect(result).toEqual({ text: raw, toolNames: [] });
  });

  it('passes through malformed JSON unchanged', () => {
    const raw = '{"content": "oops", "tool_calls": [';
    const result = unwrapToolCallEnvelope(raw);
    expect(result).toEqual({ text: raw, toolNames: [] });
  });

  it('passes through a JSON array unchanged (not a plain object)', () => {
    const raw = '[1, 2, 3]';
    const result = unwrapToolCallEnvelope(raw);
    expect(result).toEqual({ text: raw, toolNames: [] });
  });

  it('extracts an empty content string, still surfacing tool names', () => {
    const raw = JSON.stringify({
      content: '',
      tool_calls: [{ id: 'c1', name: 'save_workflow', arguments: '{}' }],
    });
    const result = unwrapToolCallEnvelope(raw);
    expect(result).toEqual({ text: '', toolNames: ['save_workflow'] });
  });

  it('returns an empty string for null/undefined-ish input without throwing', () => {
    expect(unwrapToolCallEnvelope('')).toEqual({ text: '', toolNames: [] });
    // Defensive runtime guard against a non-string value that could arrive
    // from a loosely-typed message payload.
    // @ts-expect-error — intentionally passing a non-string to exercise the guard.
    const result = unwrapToolCallEnvelope(null);
    expect(result).toEqual({ text: '', toolNames: [] });
  });

  it('preserves nested JSON inside `content` as opaque text', () => {
    const raw = JSON.stringify({
      content: '{"nested": "value"}',
      tool_calls: [{ id: 'c1', name: 'dry_run_workflow', arguments: '{}' }],
    });
    const result = unwrapToolCallEnvelope(raw);
    expect(result.text).toBe('{"nested": "value"}');
    expect(result.toolNames).toEqual(['dry_run_workflow']);
  });

  it('extracts every tool name when multiple tool_calls are present', () => {
    const raw = JSON.stringify({
      content: 'Working on it.',
      tool_calls: [
        { id: 'c1', name: 'dry_run_workflow', arguments: '{}' },
        { id: 'c2', name: 'save_workflow', arguments: '{}' },
      ],
    });
    const result = unwrapToolCallEnvelope(raw);
    expect(result.toolNames).toEqual(['dry_run_workflow', 'save_workflow']);
  });

  it('unwraps an envelope that also carries a reasoning_content key', () => {
    const raw = JSON.stringify({
      content: 'Here is the plan.',
      tool_calls: [{ id: 'c1', name: 'propose_workflow', arguments: '{}' }],
      reasoning_content: 'internal chain of thought',
    });
    const result = unwrapToolCallEnvelope(raw);
    expect(result.text).toBe('Here is the plan.');
    expect(result.toolNames).toEqual(['propose_workflow']);
  });

  it('unwraps a tool-only envelope with `content` missing entirely, surfacing tool names with empty text', () => {
    const raw = JSON.stringify({ tool_calls: [{ id: 'c1', name: 'x', arguments: '{}' }] });
    const result = unwrapToolCallEnvelope(raw);
    expect(result).toEqual({ text: '', toolNames: ['x'] });
  });

  it('unwraps a tool-only envelope with `content: null`, surfacing tool names with empty text', () => {
    const raw = JSON.stringify({
      content: null,
      tool_calls: [{ id: 'c1', name: 'save_workflow', arguments: '{}' }],
    });
    const result = unwrapToolCallEnvelope(raw);
    expect(result).toEqual({ text: '', toolNames: ['save_workflow'] });
  });

  it('passes through when `content` is present but non-string (and not null)', () => {
    const raw = JSON.stringify({ content: 42, tool_calls: [] });
    const result = unwrapToolCallEnvelope(raw);
    expect(result).toEqual({ text: raw, toolNames: [] });
  });

  it('passes through content-only JSON that lacks a `tool_calls` array (not an envelope)', () => {
    const raw = JSON.stringify({ content: 'hi' });
    const result = unwrapToolCallEnvelope(raw);
    expect(result).toEqual({ text: raw, toolNames: [] });
  });

  it('passes through when `tool_calls` is present but not an array', () => {
    const raw = JSON.stringify({ content: 'hi', tool_calls: 'nope' });
    const result = unwrapToolCallEnvelope(raw);
    expect(result).toEqual({ text: raw, toolNames: [] });
  });

  it('handles a `tool_calls` array with non-string/missing names gracefully', () => {
    const raw = JSON.stringify({
      content: 'hi',
      tool_calls: [{ id: 'c1' }, { id: 'c2', name: 42 }, { id: 'c3', name: 'ok_tool' }],
    });
    const result = unwrapToolCallEnvelope(raw);
    expect(result.text).toBe('hi');
    expect(result.toolNames).toEqual(['ok_tool']);
  });
});
