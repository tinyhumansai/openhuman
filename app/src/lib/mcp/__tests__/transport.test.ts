import type { Socket } from 'socket.io-client';
import { beforeEach, describe, expect, test } from 'vitest';

import { SocketIOMCPTransportImpl } from '../transport';
import type { MCPRequest } from '../types';

/**
 * Minimal stand-in for a `socket.io-client` Socket — just enough for the
 * transport to register/unregister listeners, emit events, and simulate
 * connect/disconnect transitions.
 */
class FakeSocket {
  connected = true;
  private listeners = new Map<string, Set<(...args: unknown[]) => void>>();
  emitted: Array<{ event: string; data: unknown }> = [];

  on(event: string, handler: (...args: unknown[]) => void) {
    let set = this.listeners.get(event);
    if (!set) {
      set = new Set();
      this.listeners.set(event, set);
    }
    set.add(handler);
  }

  off(event: string, handler: (...args: unknown[]) => void) {
    this.listeners.get(event)?.delete(handler);
  }

  emit(event: string, data: unknown) {
    this.emitted.push({ event, data });
  }

  /** Fire a socket-side event (simulating the server sending something). */
  trigger(event: string, ...args: unknown[]) {
    const set = this.listeners.get(event);
    if (!set) return;
    for (const handler of Array.from(set)) handler(...args);
  }

  asSocket(): Socket {
    return this as unknown as Socket;
  }
}

function baseRequest(id: number, method = 'tools/list'): MCPRequest {
  return { jsonrpc: '2.0', id, method };
}

describe('SocketIOMCPTransportImpl', () => {
  let socket: FakeSocket;
  let transport: SocketIOMCPTransportImpl;

  beforeEach(() => {
    socket = new FakeSocket();
    transport = new SocketIOMCPTransportImpl(socket.asSocket());
  });

  test('delivers a matching response to the in-flight request', async () => {
    const pending = transport.request(baseRequest(1));
    socket.trigger('mcp:response', { jsonrpc: '2.0', id: 1, result: { ok: true } });
    await expect(pending).resolves.toMatchObject({ id: 1, result: { ok: true } });
  });

  test('rejects pending requests when the socket emits disconnect', async () => {
    const pending = transport.request(baseRequest(42, 'tools/call'));

    // The promise is pending; simulate a socket drop.
    socket.connected = false;
    socket.trigger('disconnect', 'transport close');

    await expect(pending).rejects.toThrow(/Socket disconnected: transport close/);
  });

  test('rejects all pending requests, not just the first', async () => {
    const p1 = transport.request(baseRequest(1));
    const p2 = transport.request(baseRequest(2));
    const p3 = transport.request(baseRequest(3));

    socket.trigger('disconnect');

    await expect(p1).rejects.toThrow(/Socket disconnected/);
    await expect(p2).rejects.toThrow(/Socket disconnected/);
    await expect(p3).rejects.toThrow(/Socket disconnected/);
  });

  test('updateSocket rejects pending requests emitted on the old socket', async () => {
    const pending = transport.request(baseRequest(7));

    const replacement = new FakeSocket();
    transport.updateSocket(replacement.asSocket());

    await expect(pending).rejects.toThrow(/Socket replaced/);
  });

  test('after disconnect the handler map is empty and a replayed response is a noop', async () => {
    const pending = transport.request(baseRequest(99));
    socket.trigger('disconnect');
    await expect(pending).rejects.toThrow(/Socket disconnected/);

    // A late-arriving response for the same id must not blow up or resolve
    // anything — it should just be logged as unhandled.
    expect(() =>
      socket.trigger('mcp:response', { jsonrpc: '2.0', id: 99, result: { late: true } })
    ).not.toThrow();
  });
});
