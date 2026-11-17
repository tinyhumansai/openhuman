import { describe, expect, it } from 'vitest';

import {
  type ChatDoneEvent,
  decodeApprovalDecided,
  decodeRunModeChanged,
  externalTransferProvider,
  type TurnUsageWire,
  type WireEvent,
} from './chatWire';
import type { WebChannelEvent } from './chatWire.generated';

describe('chatWire', () => {
  it('decodeRunModeChanged lifts the wire message into a mode', () => {
    const wire: WireEvent<'run_mode_changed'> = {
      thread_id: 't',
      request_id: '',
      message: 'build',
    };
    expect(decodeRunModeChanged(wire)).toEqual({ thread_id: 't', request_id: '', mode: 'build' });
  });

  it('decodeRunModeChanged rejects a mode the UI does not know', () => {
    expect(decodeRunModeChanged({ thread_id: 't', request_id: '', message: 'x' })).toBeNull();
    expect(decodeRunModeChanged({ thread_id: 't', request_id: '' })).toBeNull();
  });

  it('decodeApprovalDecided keeps the wire fields and adds resolution', () => {
    const wire: WireEvent<'approval_decided'> = {
      thread_id: 't',
      request_id: 'r',
      message: 'allow',
      cancel_reason: 'cancelled',
    };
    expect(decodeApprovalDecided(wire)).toEqual({ ...wire, resolution: 'cancelled' });
    expect(decodeApprovalDecided({ thread_id: 't', request_id: 'r' }).resolution).toBeUndefined();
  });

  it('externalTransferProvider reads the egress descriptor slug from args', () => {
    expect(
      externalTransferProvider({
        thread_id: 't',
        request_id: 'r',
        args: { provider_slug: 'gmail', service: 'send' },
      })
    ).toBe('gmail');
    expect(externalTransferProvider({ thread_id: 't', request_id: 'r' })).toBeUndefined();
    expect(
      externalTransferProvider({ thread_id: 't', request_id: 'r', args: { provider_slug: 4 } })
    ).toBeUndefined();
  });

  it('derived event types accept a schema-shaped chat_done and usage payload', () => {
    const usage: TurnUsageWire = {
      input_tokens: 10,
      output_tokens: 5,
      cached_input_tokens: 2,
      context_tokens: 0,
      context_window: 1000,
      cost_usd: null,
      cost_source: 'unknown',
    };
    const done: ChatDoneEvent = { thread_id: 't', request_id: 'r', full_response: 'hi', usage };
    // The generated union is the source of truth: a derived event, tagged,
    // is assignable to it.
    const wire: WebChannelEvent = { ...done, event: 'chat_done', client_id: 'c' };
    expect(wire.event).toBe('chat_done');
    expect(done.usage?.cost_source).toBe('unknown');
  });
});
