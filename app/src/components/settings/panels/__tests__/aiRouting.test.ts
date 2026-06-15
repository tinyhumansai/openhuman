import { describe, expect, it } from 'vitest';

import type { CloudProvider, ProviderRef, RoutingMap } from '../AIPanel';
import {
  isChatSelectableLocalModel,
  routingWithProviderRemoved,
  toSelectableChatModels,
} from '../aiRouting';

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
const claudeCodeRef = (model = 'sonnet'): ProviderRef => ({ kind: 'claude-code', model });
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

  it('resets claude-code refs when the Claude Code provider is removed', () => {
    // Claude Code refs carry no providerSlug — they must be matched by the
    // well-known `claude-code` slug, else disconnecting leaves chats pinned to
    // `claude-code:<model>` which the Rust factory still honors.
    const routing = routingOf({ chat: claudeCodeRef('sonnet'), agentic: claudeCodeRef('opus') });
    const next = routingWithProviderRemoved(
      routing,
      { slug: 'claude-code', isLocalRuntime: false },
      []
    );
    expect(next.chat).toEqual({ kind: 'default' });
    expect(next.agentic).toEqual({ kind: 'default' });
  });

  it('leaves claude-code refs untouched when a different provider is removed', () => {
    const routing = routingOf({ chat: claudeCodeRef('sonnet'), reasoning: cloudRef('openai') });
    const next = routingWithProviderRemoved(routing, { slug: 'openai', isLocalRuntime: false }, []);
    expect(next.chat).toEqual(claudeCodeRef('sonnet'));
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

describe('isChatSelectableLocalModel (TAURI-RUST-4P6)', () => {
  it('hides models the core flagged embedding-only (chat_capable=false)', () => {
    expect(isChatSelectableLocalModel({ chat_capable: false })).toBe(false);
  });

  it('keeps chat-capable models (chat_capable=true)', () => {
    expect(isChatSelectableLocalModel({ chat_capable: true })).toBe(true);
  });

  it('keeps models with unknown capability — fail-open', () => {
    // null / undefined / missing → unknown, must stay visible so an older
    // Ollama or an /api/show miss never hides a usable chat model.
    expect(isChatSelectableLocalModel({ chat_capable: null })).toBe(true);
    expect(isChatSelectableLocalModel({ chat_capable: undefined })).toBe(true);
    expect(isChatSelectableLocalModel({})).toBe(true);
  });

  it('filters an installed-model list down to chat-capable + unknown', () => {
    const models = [
      { name: 'llama3', chat_capable: true },
      { name: 'bge-m3', chat_capable: false },
      { name: 'mystery', chat_capable: null },
    ];
    expect(models.filter(isChatSelectableLocalModel).map(m => m.name)).toEqual([
      'llama3',
      'mystery',
    ]);
  });
});

describe('toSelectableChatModels (TAURI-RUST-4P6)', () => {
  it('drops embedding-only models and maps the rest to picker shape', () => {
    const out = toSelectableChatModels([
      { name: 'llama3:8b', size: 4_700_000_000, chat_capable: true },
      { name: 'bge-m3:latest', size: 1_200_000_000, chat_capable: false },
      { name: 'mystery', size: 100, chat_capable: null },
    ]);
    expect(out).toEqual([
      { id: 'llama3:8b', sizeBytes: 4_700_000_000, family: 'llama3' },
      { id: 'mystery', sizeBytes: 100, family: 'mystery' },
    ]);
  });

  it('derives family from the name before the first `:` or `/` separator', () => {
    const out = toSelectableChatModels([
      { name: 'qwen2.5:14b', chat_capable: true },
      { name: 'library/phi3', chat_capable: true },
    ]);
    expect(out.map(m => m.family)).toEqual(['qwen2.5', 'library']);
  });

  it('defaults sizeBytes to 0 when size is null/undefined', () => {
    const out = toSelectableChatModels([
      { name: 'a', size: null, chat_capable: true },
      { name: 'b', chat_capable: true },
    ]);
    expect(out.map(m => m.sizeBytes)).toEqual([0, 0]);
  });

  it('returns an empty list for no installed models', () => {
    expect(toSelectableChatModels([])).toEqual([]);
  });
});
