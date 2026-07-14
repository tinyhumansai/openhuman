/**
 * runStepSummary (issue B20) — plain-language step summary derivation.
 *
 * `t` is a trivial passthrough stub here since the summarizer takes a `t`
 * function rather than calling `useT()` itself (see module doc); the real
 * strings live in `lib/i18n/en.ts` under `flowRuns.inspector.summary.*`.
 */
import { describe, expect, it } from 'vitest';

import { normalizeItems } from './runItems';
import { summarizeStep } from './runStepSummary';

const STRINGS: Record<string, string> = {
  'flowRuns.inspector.summary.failedPrefix': "Couldn't complete:",
  'flowRuns.inspector.summary.unknownError': 'something went wrong',
  'flowRuns.inspector.summary.itemsFetched': 'Fetched {count} item(s)',
  'flowRuns.inspector.summary.completed': 'Step completed',
  'flowRuns.inspector.summary.noOutput': 'No output produced',
};
const t = (key: string) => STRINGS[key] ?? key;

describe('summarizeStep', () => {
  describe('success cases', () => {
    it('surfaces a payload-provided `summary` string verbatim as the primary line', () => {
      const items = normalizeItems([
        { json: { has_important: false, summary: 'No new emails today.' } },
      ]);
      const result = summarizeStep({ status: 'success' }, items, t);
      expect(result).toEqual({ outcome: 'success', text: 'No new emails today.' });
    });

    it('falls back to a `message` field when there is no `summary`', () => {
      const items = normalizeItems([{ json: { message: 'Draft saved.' } }]);
      expect(summarizeStep({ status: 'success' }, items, t)).toEqual({
        outcome: 'success',
        text: 'Draft saved.',
      });
    });

    it('reports a count for an array payload, generic across any tool', () => {
      const items = normalizeItems([{ json: Array.from({ length: 20 }, (_, i) => ({ id: i })) }]);
      expect(summarizeStep({ status: 'success' }, items, t)).toEqual({
        outcome: 'success',
        text: 'Fetched 20 item(s)',
      });
    });

    it('reports a count for a Composio-style `{ data: [...] }` envelope', () => {
      const items = normalizeItems([
        { json: { data: [{ id: 1 }, { id: 2 }, { id: 3 }], successful: true, costUsd: 0.001 } },
      ]);
      expect(summarizeStep({ status: 'success' }, items, t)).toEqual({
        outcome: 'success',
        text: 'Fetched 3 item(s)',
      });
    });

    it('reports an item count via itemsFetched for multiple items (no separate itemsProduced branch — primaryPayload always turns >1 items into an array, so arrayLength always wins)', () => {
      const items = normalizeItems([{ json: { id: 1 } }, { json: { id: 2 } }, { json: { id: 3 } }]);
      expect(summarizeStep({ status: 'success' }, items, t)).toEqual({
        outcome: 'success',
        text: 'Fetched 3 item(s)',
      });
    });

    it('falls back to a generic "Step completed" when nothing more specific is known', () => {
      const items = normalizeItems([{ json: { internalId: 'abc123' } }]);
      expect(summarizeStep({ status: 'success' }, items, t)).toEqual({
        outcome: 'success',
        text: 'Step completed',
      });
    });

    it('reports neutral "No output produced" when the step produced no items', () => {
      expect(summarizeStep({ status: 'success' }, [], t)).toEqual({
        outcome: 'neutral',
        text: 'No output produced',
      });
    });
  });

  describe('failure cases', () => {
    it('reports failure with the payload error message when step.status is "error"', () => {
      const items = normalizeItems([{ json: { error: 'recipient address invalid' } }]);
      expect(summarizeStep({ status: 'error' }, items, t)).toEqual({
        outcome: 'error',
        text: "Couldn't complete: recipient address invalid",
      });
    });

    it('treats a Composio `successful: false` envelope as a failure even when step.status is "success"', () => {
      // The engine step itself didn't throw (status: success), but the
      // underlying tool call failed — a distinct, generic failure signal.
      const items = normalizeItems([
        { json: { data: null, successful: false, error: 'invalid recipient' } },
      ]);
      expect(summarizeStep({ status: 'success' }, items, t)).toEqual({
        outcome: 'error',
        text: "Couldn't complete: invalid recipient",
      });
    });

    it('unwraps a nested `{ error: { message } }` shape', () => {
      const items = normalizeItems([{ json: { error: { message: 'quota exceeded' } } }]);
      expect(summarizeStep({ status: 'error' }, items, t)).toEqual({
        outcome: 'error',
        text: "Couldn't complete: quota exceeded",
      });
    });

    it('falls back to a generic reason when no error message is present', () => {
      const items = normalizeItems([{ json: {} }]);
      expect(summarizeStep({ status: 'error' }, items, t)).toEqual({
        outcome: 'error',
        text: "Couldn't complete: something went wrong",
      });
    });

    it('never surfaces internal flow:/run: ids in the summary text', () => {
      const items = normalizeItems([
        { json: { error: 'send failed', flow_id: 'flow-secret-1', run_id: 'run-secret-2' } },
      ]);
      const result = summarizeStep({ status: 'error' }, items, t);
      expect(result.text).not.toContain('flow-secret-1');
      expect(result.text).not.toContain('run-secret-2');
    });
  });

  describe('status-less payloads (reconstructed steps, e.g. triggers)', () => {
    it('does not infer failure from a bare top-level `error` field when status is absent', () => {
      // Trigger/webhook payloads can be reconstructed without a persisted
      // `status` while still carrying arbitrary user data — a field that
      // happens to be named "error" here is just payload content, not a
      // failure signal.
      const items = normalizeItems([{ json: { event: 'x', error: 'user typo field' } }]);
      const result = summarizeStep({}, items, t);
      expect(result.outcome).not.toBe('error');
    });

    it('still treats a known Composio `successful: false` envelope as failed even without status', () => {
      const items = normalizeItems([
        { json: { data: null, successful: false, error: 'invalid recipient' } },
      ]);
      expect(summarizeStep({}, items, t)).toEqual({
        outcome: 'error',
        text: "Couldn't complete: invalid recipient",
      });
    });
  });
});
