import { describe, expect, it } from 'vitest';

import type { CloudProvider, ProviderRef, RoutingMap } from '../AIPanel';
import { routingWithProviderRemoved } from '../aiRouting';

const WORKLOADS = [
  'chat',
  'reasoning',
  'agentic',
  'coding',
  'memory',
  'heartbeat',
  'learning',
  'subconscious',
] as const;

/** Build a full 8-workload routing map defaulting every slot to managed. */
function routingOf(
  overrides: Partial<Record<(typeof WORKLOADS)[number], ProviderRef>>
): RoutingMap {
  const base = Object.fromEntries(
    WORKLOADS.map(w => [w, { kind: 'default' } as ProviderRef])
  ) as RoutingMap;
  return { ...base, ...overrides };
}

const cloudRef = (slug: string, model = 'm'): ProviderRef => ({
  kind: 'cloud',
  providerSlug: slug,
  model,
});
const localRef = (model = 'llama3'): ProviderRef => ({ kind: 'local', model });
// The helper only reads `.slug`; the rest of CloudProvider is irrelevant here.
const provider = (slug: string): CloudProvider =>
  ({
    id: `id-${slug}`,
    slug,
    label: slug,
    endpoint: '',
    maskedKey: '',
  }) as unknown as CloudProvider;

describe('routingWithProviderRemoved', () => {
  it('resets workloads pinned to a removed cloud provider back to default', () => {
    const routing = routingOf({ chat: cloudRef('openrouter'), coding: cloudRef('openrouter') });
    const next = routingWithProviderRemoved(
      routing,
      { slug: 'openrouter', isLocalRuntime: false },
      []
    );
    expect(next.chat).toEqual({ kind: 'default' });
    expect(next.coding).toEqual({ kind: 'default' });
  });

  it('leaves a different cloud provider untouched when one is removed', () => {
    const routing = routingOf({ chat: cloudRef('openrouter'), reasoning: cloudRef('openai') });
    const next = routingWithProviderRemoved(
      routing,
      { slug: 'openrouter', isLocalRuntime: false },
      [provider('openai')]
    );
    expect(next.chat).toEqual({ kind: 'default' });
    expect(next.reasoning).toEqual(cloudRef('openai'));
  });

  it('resets local-runtime refs when the last local runtime is disabled', () => {
    const routing = routingOf({ chat: localRef('llama3'), agentic: localRef('llama3') });
    // Disabling ollama with no local runtime remaining → local refs are orphaned.
    const next = routingWithProviderRemoved(routing, { slug: 'ollama', isLocalRuntime: true }, []);
    expect(next.chat).toEqual({ kind: 'default' });
    expect(next.agentic).toEqual({ kind: 'default' });
  });

  it('keeps local refs when another local runtime is still enabled', () => {
    const routing = routingOf({ chat: localRef('llama3') });
    // Disabling lmstudio while ollama remains → the local ref may resolve to ollama.
    const next = routingWithProviderRemoved(routing, { slug: 'lmstudio', isLocalRuntime: true }, [
      provider('ollama'),
    ]);
    expect(next.chat).toEqual(localRef('llama3'));
  });

  it('does not scrub local refs when a cloud provider is removed (regression guard)', () => {
    const routing = routingOf({ chat: localRef('llama3'), reasoning: cloudRef('openrouter') });
    const next = routingWithProviderRemoved(
      routing,
      { slug: 'openrouter', isLocalRuntime: false },
      []
    );
    expect(next.chat).toEqual(localRef('llama3'));
    expect(next.reasoning).toEqual({ kind: 'default' });
  });

  it('preserves all 8 workload slots', () => {
    const next = routingWithProviderRemoved(
      routingOf({}),
      { slug: 'x', isLocalRuntime: false },
      []
    );
    expect(Object.keys(next).sort()).toEqual([...WORKLOADS].sort());
  });
});
