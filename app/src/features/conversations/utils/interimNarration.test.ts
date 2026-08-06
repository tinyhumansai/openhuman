import { describe, expect, it } from 'vitest';

import type { ThreadMessage } from '../../../types/thread';
import { supersededInterimIndexes } from './interimNarration';

function msg(
  sender: 'user' | 'agent',
  content: string,
  extraMetadata?: Record<string, unknown>
): ThreadMessage {
  return {
    id: `${sender}-${content.slice(0, 8)}-${Math.random().toString(36).slice(2, 8)}`,
    sender,
    content,
    createdAt: new Date(0).toISOString(),
    ...(extraMetadata ? { extraMetadata } : {}),
  } as unknown as ThreadMessage;
}

const interim = (text: string) => msg('agent', text, { isInterim: true, requestId: 'r1' });
const answer = (text: string) => msg('agent', text, { citations: [] });

describe('supersededInterimIndexes', () => {
  it('hides narration once the turn produced its answer', () => {
    const messages = [
      msg('user', 'how many goals?'),
      interim('Let me get the data for both.'),
      interim('The HTML is hard to parse. Let me search for a clean table.'),
      answer('He scored 11 goals.'),
    ];
    expect([...supersededInterimIndexes(messages)].sort()).toEqual([1, 2]);
  });

  it('keeps narration while the turn is still in flight (no answer yet)', () => {
    const messages = [
      msg('user', 'how many goals?'),
      interim('Let me get the data for both.'),
      interim('Let me get a cleaner source.'),
    ];
    expect(supersededInterimIndexes(messages).size).toBe(0);
  });

  // A turn that errored before answering: its narration is the only record of
  // what actually ran, so it must survive.
  it('keeps narration for a turn that died before answering', () => {
    const messages = [
      msg('user', 'first question'),
      interim('Let me check.'),
      answer('Here is the answer.'),
      msg('user', 'second question'),
      interim('Let me search.'),
    ];
    // Only the FIRST turn's narration is superseded.
    expect([...supersededInterimIndexes(messages)]).toEqual([1]);
  });

  it('scopes per turn across a multi-turn thread', () => {
    const messages = [
      msg('user', 'q1'),
      interim('n1'),
      answer('a1'),
      msg('user', 'q2'),
      interim('n2'),
      interim('n3'),
      answer('a2'),
    ];
    expect([...supersededInterimIndexes(messages)].sort((a, b) => a - b)).toEqual([1, 4, 5]);
  });

  it('never hides real answers or user messages', () => {
    const messages = [msg('user', 'q'), interim('n'), answer('a')];
    const hidden = supersededInterimIndexes(messages);
    expect(hidden.has(0)).toBe(false);
    expect(hidden.has(2)).toBe(false);
  });

  it('handles an empty thread and an interim-only thread with no user message', () => {
    expect(supersededInterimIndexes([]).size).toBe(0);
    // Proactive-only run: narration with no answer yet stays visible.
    expect(supersededInterimIndexes([interim('working…')]).size).toBe(0);
    // Proactive-only run that did answer: narration is superseded.
    expect([...supersededInterimIndexes([interim('working…'), answer('done')])]).toEqual([0]);
  });
});
