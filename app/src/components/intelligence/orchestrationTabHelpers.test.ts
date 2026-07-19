import { describe, expect, it } from 'vitest';

import type { ContactView } from '../../lib/agentworld/invokeApiClient';
import type { ChatWindow } from '../../lib/orchestration/useOrchestrationChats';
import {
  acceptedContactIds,
  chatTime,
  contactAddress,
  contactBadgeKey,
  extractHandle,
  formatTime,
  pendingContactIds,
  truncate,
} from './orchestrationTabHelpers';

const contact = (over: Partial<ContactView>): ContactView =>
  ({ agentId: '', status: 'accepted', direction: 'incoming', ...over }) as ContactView;

const chat = (over: Partial<ChatWindow>): ChatWindow =>
  ({ id: 'c', pinned: false, peerAgentId: undefined, ...over }) as ChatWindow;

describe('orchestrationTabHelpers', () => {
  it('formatTime returns empty for null/invalid and formats a real timestamp', () => {
    expect(formatTime(null)).toBe('');
    expect(formatTime('not-a-date')).toBe('');
    expect(formatTime('2026-07-01T12:00:00.000Z')).not.toBe('');
  });

  it('truncate leaves short text and ellipsises long text within the cap', () => {
    expect(truncate('short', 96)).toBe('short');
    const out = truncate('x'.repeat(50), 10);
    expect(out).toHaveLength(10);
    expect(out.endsWith('…')).toBe(true);
  });

  it('chatTime parses a timestamp and falls back to 0', () => {
    expect(chatTime(chat({ lastTimestamp: null }))).toBe(0);
    expect(chatTime(chat({ lastTimestamp: 'nope' }))).toBe(0);
    expect(chatTime(chat({ lastTimestamp: '2026-07-01T12:00:00.000Z' }))).toBeGreaterThan(0);
  });

  it('contactAddress prefers agentId then the contact record by direction', () => {
    expect(contactAddress(contact({ agentId: '@a' }))).toBe('@a');
    expect(
      contactAddress(
        contact({ agentId: '', direction: 'incoming', contact: { requester: '@req' } as never })
      )
    ).toBe('@req');
    expect(
      contactAddress(
        contact({ agentId: '', direction: 'outgoing', contact: { addressee: '@to' } as never })
      )
    ).toBe('@to');
    expect(contactAddress(contact({ agentId: '' }))).toBe('');
  });

  it('accepted/pending id sets derive from the contact records', () => {
    const accepted = acceptedContactIds([
      contact({ agentId: '@ok', status: 'accepted' }),
      contact({ agentId: '@no', status: 'pending' }),
    ]);
    expect(accepted.has('@ok')).toBe(true);
    expect(accepted.has('@no')).toBe(false);

    const pending = pendingContactIds({
      incoming: [contact({ agentId: '@p', status: 'pending' })],
      outgoing: [],
    } as never);
    expect(pending.has('@p')).toBe(true);
  });

  it('contactBadgeKey maps a session chat to linked/pending/unlinked', () => {
    const accepted = new Set(['@a']);
    const pending = new Set(['@b']);
    expect(contactBadgeKey(chat({ pinned: true }), accepted, pending)).toBeNull();
    expect(contactBadgeKey(chat({ peerAgentId: '@a' }), accepted, pending)).toBe(
      'tinyplaceOrchestration.pairing.linked'
    );
    expect(contactBadgeKey(chat({ peerAgentId: '@b' }), accepted, pending)).toBe(
      'tinyplaceOrchestration.pairing.pending'
    );
    expect(contactBadgeKey(chat({ peerAgentId: '@c' }), accepted, pending)).toBe(
      'tinyplaceOrchestration.pairing.unlinked'
    );
  });

  it('extractHandle finds a username from agents or identities and strips @', () => {
    expect(extractHandle({ agents: [{ username: '@nick' }] })).toBe('nick');
    expect(extractHandle({ identities: [{ username: 'openhuman' }] })).toBe('openhuman');
    expect(extractHandle({})).toBeNull();
  });
});
