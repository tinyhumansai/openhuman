/**
 * Test doubles for the live voice stack: a scriptable WebSocket and a minimal
 * AudioContext whose clock the test drives. Test-only — imported from
 * `*.test.ts(x)` files only.
 */
import { vi } from 'vitest';

export class FakeWebSocket {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;
  static instances: FakeWebSocket[] = [];

  readonly url: string;
  readyState = FakeWebSocket.CONNECTING;
  binaryType = 'blob';
  sent: Array<string | ArrayBuffer> = [];
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: unknown }) => void) | null = null;
  onerror: (() => void) | null = null;
  onclose: ((event: { code: number; reason: string; wasClean: boolean }) => void) | null = null;

  constructor(url: string) {
    this.url = url;
    FakeWebSocket.instances.push(this);
  }

  send(data: string | ArrayBuffer): void {
    this.sent.push(data);
  }

  close(code = 1000, reason = ''): void {
    if (this.readyState === FakeWebSocket.CLOSED) return;
    this.readyState = FakeWebSocket.CLOSED;
    this.onclose?.({ code, reason, wasClean: true });
  }

  // ── test drivers ──
  open(): void {
    this.readyState = FakeWebSocket.OPEN;
    this.onopen?.();
  }

  emit(event: Record<string, unknown>): void {
    this.onmessage?.({ data: JSON.stringify(event) });
  }

  emitRaw(data: unknown): void {
    this.onmessage?.({ data });
  }

  drop(code = 1006): void {
    this.readyState = FakeWebSocket.CLOSED;
    this.onclose?.({ code, reason: '', wasClean: false });
  }

  jsonFrames(): Array<Record<string, unknown>> {
    return this.sent
      .filter((d): d is string => typeof d === 'string')
      .map(d => JSON.parse(d) as Record<string, unknown>);
  }

  static latest(): FakeWebSocket {
    const ws = FakeWebSocket.instances[FakeWebSocket.instances.length - 1];
    if (!ws) throw new Error('no FakeWebSocket opened');
    return ws;
  }
}

export interface FakeSource {
  buffer: { length: number } | null;
  started: number | null;
  stopped: boolean;
  onended: (() => void) | null;
  connect: ReturnType<typeof vi.fn>;
  disconnect: ReturnType<typeof vi.fn>;
  start: (when: number) => void;
  stop: () => void;
}

export class FakeAudioContext {
  currentTime = 0;
  state: 'running' | 'suspended' | 'closed' = 'running';
  sampleRate = 48_000;
  destination = {};
  sources: FakeSource[] = [];
  close = vi.fn(async () => {
    this.state = 'closed';
  });
  resume = vi.fn(async () => {
    this.state = 'running';
  });
  audioWorklet = { addModule: vi.fn(async () => undefined) };

  createBuffer(_channels: number, length: number, _rate: number) {
    const data = new Float32Array(length);
    return { length, getChannelData: () => data };
  }

  createBufferSource(): FakeSource {
    const source: FakeSource = {
      buffer: null,
      started: null,
      stopped: false,
      onended: null,
      connect: vi.fn(),
      disconnect: vi.fn(),
      start(when: number) {
        source.started = when;
      },
      stop() {
        source.stopped = true;
      },
    };
    this.sources.push(source);
    return source;
  }

  createMediaStreamSource() {
    return { connect: vi.fn(), disconnect: vi.fn() };
  }

  /** Finish every source whose slot ends at or before `time`. */
  advanceTo(time: number): void {
    this.currentTime = time;
  }
}

/** PCM16LE bytes for `count` samples of a constant value in [-1, 1). */
export function pcmBytes(count: number, value = 0.5): ArrayBuffer {
  const buf = new ArrayBuffer(count * 2);
  const view = new DataView(buf);
  for (let i = 0; i < count; i += 1) view.setInt16(i * 2, Math.round(value * 0x7fff), true);
  return buf;
}
