import { describe, expect, it } from 'vitest';

import type { RegistryBridgeErrorMeta } from '../../services/api/coreRegistriesClient';
import { createRegistryInspectionState, LOAD_MORE_LIMIT, registryInspectionReducer } from './state';

const agentSummary = {
  id: 'agent-row',
  agentKey: 'agent.alpha',
  version: 7,
  lifecycleState: 'active' as const,
  configurationFingerprint: 'a'.repeat(64),
  ownerActorType: 'service' as const,
  ownerActorId: 'registry-reader',
  createdAt: '2026-09-01T12:00:00Z',
};

const agentDetail = {
  ...agentSummary,
  configuration: {
    schemaVersion: 1,
    domainKey: 'ops',
    owner: { actorType: 'service' as const, actorId: 'registry-reader' },
    allowedToolRefs: [{ toolKey: 'tool.alpha', version: 3 }],
    knowledgeScopeRefs: [],
    riskPolicyRef: null,
  },
};

const toolDefinitionSummary = {
  toolKey: 'tool.alpha',
  version: 3,
  lifecycleState: 'active' as const,
  definitionFingerprint: 'b'.repeat(64),
  schemaVersion: 1,
  displayName: 'Tool Alpha',
  description: 'Reads data',
  toolEffectClass: 'read_only' as const,
  abstractAuthScopes: ['scope.read'],
  createdAt: '2026-09-01T12:05:00Z',
};

const toolEnablement = {
  toolKey: 'tool.alpha',
  version: 5,
  lifecycleState: 'enabled' as const,
  generation: 12,
  timeoutCapMs: 5000,
  approvalRequired: false,
  allowTtlSeconds: null,
  auditMode: 'metadata_only' as const,
  updatedAt: '2026-09-01T12:06:00Z',
};

function httpError(httpStatus: number, coreCode: string): RegistryBridgeErrorMeta {
  return { kind: 'YouPetCoreHttpError', httpStatus, coreCode };
}

describe('registry inspection state', () => {
  it('keeps independent collection windows and leaves Tool Enablements unpaged', () => {
    let state = createRegistryInspectionState();

    state = registryInspectionReducer(state, {
      type: 'tab_request_started',
      tab: 'agents',
      generation: 1,
    });
    state = registryInspectionReducer(state, {
      type: 'cursor_collection_request_succeeded',
      tab: 'agents',
      collection: 'agents',
      generation: 1,
      items: [agentSummary],
      nextCursor: 'agent-cursor-1',
      append: false,
      observedAt: '2026-09-01T12:10:00Z',
    });

    state = registryInspectionReducer(state, {
      type: 'tab_request_started',
      tab: 'tools',
      generation: 1,
    });
    state = registryInspectionReducer(state, {
      type: 'cursor_collection_request_succeeded',
      tab: 'tools',
      collection: 'toolDefinitions',
      generation: 1,
      items: [toolDefinitionSummary],
      nextCursor: 'tool-definition-cursor-1',
      append: false,
      observedAt: '2026-09-01T12:11:00Z',
    });
    state = registryInspectionReducer(state, {
      type: 'unpaged_collection_request_succeeded',
      tab: 'tools',
      collection: 'toolEnablements',
      generation: 1,
      items: [toolEnablement],
      observedAt: '2026-09-01T12:12:00Z',
    });

    expect(state.tabs.agents.collections.agents.items).toEqual([agentSummary]);
    expect(state.tabs.agents.collections.agents.nextCursor).toBe('agent-cursor-1');
    expect(state.tabs.tools.collections.toolDefinitions.items).toEqual([toolDefinitionSummary]);
    expect(state.tabs.tools.collections.toolDefinitions.nextCursor).toBe(
      'tool-definition-cursor-1'
    );
    expect(state.tabs.tools.collections.toolEnablements.items).toEqual([toolEnablement]);
    expect('nextCursor' in state.tabs.tools.collections.toolEnablements).toBe(false);
    expect(LOAD_MORE_LIMIT).toBe(50);
  });

  it('clears detail and URL on tab switch while preserving loaded windows', () => {
    let state = createRegistryInspectionState({
      tab: 'agents',
      detail: { kind: 'agent', key: 'agent.alpha', version: 7 },
    });

    state = registryInspectionReducer(state, {
      type: 'cursor_collection_request_succeeded',
      tab: 'agents',
      collection: 'agents',
      generation: 0,
      items: [agentSummary],
      nextCursor: null,
      append: false,
      observedAt: '2026-09-01T12:20:00Z',
    });
    state = registryInspectionReducer(state, {
      type: 'detail_request_succeeded',
      tab: 'agents',
      generation: 0,
      detail: { kind: 'agent', key: 'agent.alpha', version: 7 },
      record: agentDetail,
    });
    state = registryInspectionReducer(state, {
      type: 'tab_selected',
      tab: 'tools',
      source: 'user',
    });

    expect(state.urlState).toEqual({ tab: 'tools', detail: null });
    expect(state.tabs.agents.collections.agents.items).toEqual([agentSummary]);
    expect(state.tabs.agents.detail).toEqual({ kind: 'none' });
  });

  it('rejects stale generation updates and keeps generations monotonic', () => {
    let state = createRegistryInspectionState();

    state = registryInspectionReducer(state, {
      type: 'tab_request_started',
      tab: 'agents',
      generation: 1,
    });
    state = registryInspectionReducer(state, {
      type: 'tab_request_started',
      tab: 'agents',
      generation: 2,
    });
    state = registryInspectionReducer(state, {
      type: 'cursor_collection_request_succeeded',
      tab: 'agents',
      collection: 'agents',
      generation: 1,
      items: [agentSummary],
      nextCursor: null,
      append: false,
      observedAt: '2026-09-01T12:21:00Z',
    });

    expect(state.tabs.agents.generation).toBe(2);
    expect(state.tabs.agents.collections.agents.items).toEqual([]);

    state = registryInspectionReducer(state, {
      type: 'cursor_collection_request_succeeded',
      tab: 'agents',
      collection: 'agents',
      generation: 2,
      items: [{ ...agentSummary, agentKey: 'agent.beta' }],
      nextCursor: null,
      append: false,
      observedAt: '2026-09-01T12:22:00Z',
    });

    expect(state.tabs.agents.collections.agents.items).toEqual([
      { ...agentSummary, agentKey: 'agent.beta' },
    ]);
  });

  it('allows one invalid-cursor restart per generation and blocks on repetition', () => {
    let state = createRegistryInspectionState();

    state = registryInspectionReducer(state, {
      type: 'tab_request_started',
      tab: 'tools',
      generation: 1,
    });
    state = registryInspectionReducer(state, {
      type: 'collection_request_failed',
      tab: 'tools',
      collection: 'toolDefinitions',
      generation: 1,
      error: httpError(422, 'invalid_cursor'),
      restartPlanned: true,
    });

    expect(state.tabs.tools.collections.toolDefinitions.restartGeneration).toBe(1);
    expect(state.tabs.tools.collections.toolDefinitions.items).toEqual([]);
    expect(state.tabs.tools.collections.toolDefinitions.nextCursor).toBeNull();
    expect(state.tabs.tools.collections.toolDefinitions.observation).toEqual({
      kind: 'loading',
      generation: 1,
    });

    state = registryInspectionReducer(state, {
      type: 'collection_request_failed',
      tab: 'tools',
      collection: 'toolDefinitions',
      generation: 1,
      error: httpError(422, 'invalid_cursor'),
      restartPlanned: false,
    });

    expect(state.tabs.tools.collections.toolDefinitions.observation).toEqual({
      kind: 'blocked',
      error: httpError(422, 'invalid_cursor'),
    });
    expect(state.tabs.tools.summaryState).toBe('blocked');
  });

  it('advances observedAt only on complete success, preserves stale data on transient failure, and clears the full surface for blockers', () => {
    let state = createRegistryInspectionState({ tab: 'tools', detail: null });

    state = registryInspectionReducer(state, {
      type: 'tab_request_started',
      tab: 'tools',
      generation: 1,
    });
    state = registryInspectionReducer(state, {
      type: 'cursor_collection_request_succeeded',
      tab: 'tools',
      collection: 'toolDefinitions',
      generation: 1,
      items: [toolDefinitionSummary],
      nextCursor: null,
      append: false,
      observedAt: '2026-09-01T12:30:00Z',
    });

    expect(state.tabs.tools.observedAt).toBeNull();

    state = registryInspectionReducer(state, {
      type: 'unpaged_collection_request_succeeded',
      tab: 'tools',
      collection: 'toolEnablements',
      generation: 1,
      items: [toolEnablement],
      observedAt: '2026-09-01T12:31:00Z',
    });

    expect(state.tabs.tools.observedAt).toBe('2026-09-01T12:31:00Z');
    expect(state.tabs.tools.summaryState).toBe('fresh');

    state = registryInspectionReducer(state, {
      type: 'tab_request_started',
      tab: 'tools',
      generation: 2,
    });
    state = registryInspectionReducer(state, {
      type: 'cursor_collection_request_succeeded',
      tab: 'tools',
      collection: 'toolDefinitions',
      generation: 2,
      items: [{ ...toolDefinitionSummary, version: 4 }],
      nextCursor: null,
      append: false,
      observedAt: '2026-09-01T12:32:00Z',
    });
    state = registryInspectionReducer(state, {
      type: 'collection_request_failed',
      tab: 'tools',
      collection: 'toolEnablements',
      generation: 2,
      error: httpError(429, 'rate_limited'),
      restartPlanned: false,
    });

    expect(state.tabs.tools.observedAt).toBe('2026-09-01T12:31:00Z');
    expect(state.tabs.tools.summaryState).toBe('partial');
    expect(state.tabs.tools.collections.toolEnablements.items).toEqual([toolEnablement]);
    expect(state.tabs.tools.collections.toolEnablements.observation).toEqual({
      kind: 'stale',
      observedAt: '2026-09-01T12:31:00Z',
      error: httpError(429, 'rate_limited'),
    });

    state = registryInspectionReducer(state, {
      type: 'surface_blocked',
      error: httpError(403, 'forbidden_actor'),
    });

    expect(state.urlState).toEqual({ tab: 'tools', detail: null });
    expect(state.surfaceError).toEqual(httpError(403, 'forbidden_actor'));
    expect(state.tabs.agents.collections.agents.items).toEqual([]);
    expect(state.tabs.tools.collections.toolDefinitions.items).toEqual([]);
    expect(state.tabs.tools.collections.toolEnablements.items).toEqual([]);
  });

  it('stores a Retry-After cooldown for 429 collection failures while retaining last-known-good stale data', () => {
    let state = createRegistryInspectionState({ tab: 'tools', detail: null });

    state = registryInspectionReducer(state, {
      type: 'tab_request_started',
      tab: 'tools',
      generation: 1,
    });
    state = registryInspectionReducer(state, {
      type: 'cursor_collection_request_succeeded',
      tab: 'tools',
      collection: 'toolDefinitions',
      generation: 1,
      items: [toolDefinitionSummary],
      nextCursor: null,
      append: false,
      observedAt: '2026-09-01T12:30:00Z',
    });
    state = registryInspectionReducer(state, {
      type: 'unpaged_collection_request_succeeded',
      tab: 'tools',
      collection: 'toolEnablements',
      generation: 1,
      items: [toolEnablement],
      observedAt: '2026-09-01T12:31:00Z',
    });

    state = registryInspectionReducer(state, {
      type: 'collection_request_started',
      tab: 'tools',
      collection: 'toolDefinitions',
      generation: 2,
    });
    state = registryInspectionReducer(state, {
      type: 'collection_request_failed',
      tab: 'tools',
      collection: 'toolDefinitions',
      generation: 2,
      error: {
        kind: 'YouPetCoreHttpError',
        httpStatus: 429,
        coreCode: 'rate_limited',
        retryAfterSeconds: 5,
      },
      restartPlanned: false,
      failedAtMs: Date.parse('2026-09-01T12:32:00Z'),
    });

    expect(state.tabs.tools.collections.toolDefinitions.items).toEqual([toolDefinitionSummary]);
    expect(state.tabs.tools.collections.toolDefinitions.observation).toEqual({
      kind: 'stale',
      observedAt: '2026-09-01T12:30:00Z',
      error: {
        kind: 'YouPetCoreHttpError',
        httpStatus: 429,
        coreCode: 'rate_limited',
        retryAfterSeconds: 5,
      },
    });
    expect(state.tabs.tools.collections.toolDefinitions.retryDisabledUntil).toBe(
      Date.parse('2026-09-01T12:32:05Z')
    );
  });

  it('preserves active Retry-After cooldowns when refresh or retry request-start actions begin', () => {
    let state = createRegistryInspectionState({ tab: 'tools', detail: null });

    state = registryInspectionReducer(state, {
      type: 'cursor_collection_request_succeeded',
      tab: 'tools',
      collection: 'toolDefinitions',
      generation: 1,
      items: [toolDefinitionSummary],
      nextCursor: 'tool-definition-cursor-1',
      append: false,
      observedAt: '2026-09-01T12:30:00Z',
    });
    state = registryInspectionReducer(state, {
      type: 'unpaged_collection_request_succeeded',
      tab: 'tools',
      collection: 'toolEnablements',
      generation: 1,
      items: [toolEnablement],
      observedAt: '2026-09-01T12:31:00Z',
    });
    state.tabs.tools.collections.toolDefinitions.retryDisabledUntil =
      Date.parse('2026-09-01T12:32:05Z');
    state.tabs.tools.collections.toolEnablements.retryDisabledUntil =
      Date.parse('2026-09-01T12:32:07Z');

    state = registryInspectionReducer(state, {
      type: 'tab_request_started',
      tab: 'tools',
      generation: 2,
    });

    expect(state.tabs.tools.collections.toolDefinitions.retryDisabledUntil).toBe(
      Date.parse('2026-09-01T12:32:05Z')
    );
    expect(state.tabs.tools.collections.toolEnablements.retryDisabledUntil).toBe(
      Date.parse('2026-09-01T12:32:07Z')
    );

    state = registryInspectionReducer(state, {
      type: 'collection_request_started',
      tab: 'tools',
      collection: 'toolDefinitions',
      generation: 3,
    });

    expect(state.tabs.tools.collections.toolDefinitions.retryDisabledUntil).toBe(
      Date.parse('2026-09-01T12:32:05Z')
    );
  });

  it('keeps the source collection and only invalidates detail on 404', () => {
    let state = createRegistryInspectionState({
      tab: 'agents',
      detail: { kind: 'agent', key: 'agent.alpha', version: 7 },
    });

    state = registryInspectionReducer(state, {
      type: 'cursor_collection_request_succeeded',
      tab: 'agents',
      collection: 'agents',
      generation: 0,
      items: [agentSummary],
      nextCursor: null,
      append: false,
      observedAt: '2026-09-01T12:40:00Z',
    });
    state = registryInspectionReducer(state, {
      type: 'detail_request_succeeded',
      tab: 'agents',
      generation: 0,
      detail: { kind: 'agent', key: 'agent.alpha', version: 7 },
      record: agentDetail,
    });

    state = registryInspectionReducer(state, {
      type: 'detail_request_failed',
      tab: 'agents',
      generation: 0,
      detail: { kind: 'agent', key: 'agent.alpha', version: 7 },
      error: httpError(404, 'agent_not_found'),
    });

    expect(state.urlState).toEqual({
      tab: 'agents',
      detail: { kind: 'agent', key: 'agent.alpha', version: 7 },
    });
    expect(state.tabs.agents.collections.agents.items).toEqual([agentSummary]);
    expect(state.tabs.agents.detail).toEqual({
      kind: 'missing',
      detail: { kind: 'agent', key: 'agent.alpha', version: 7 },
      error: httpError(404, 'agent_not_found'),
    });
  });
});
