import type {
  AgentRegistryAgent,
  AgentRegistryAgentSummary,
  ConnectorRegistryBinding,
  ConnectorRegistryBindingSummary,
  ConnectorRegistryType,
  ConnectorRegistryTypeSummary,
  RegistryBridgeErrorMeta,
  ToolRegistryToolDefinition,
  ToolRegistryToolDefinitionSummary,
  ToolRegistryToolEnablement,
} from '../../services/api/coreRegistriesClient';
import {
  type CursorRegistryCollectionState,
  type RegistryCollectionKey,
  type RegistryDetailRef,
  type RegistryInspectionState,
  type RegistryObservationState,
  type RegistrySummaryState,
  type RegistryTab,
  type RegistryUrlState,
  type UnpagedRegistryCollectionState,
} from './types';

export const LOAD_MORE_LIMIT = 50;

type AgentsTabState = RegistryInspectionState['tabs']['agents'];
type ToolsTabState = RegistryInspectionState['tabs']['tools'];
type ConnectorsTabState = RegistryInspectionState['tabs']['connectors'];

type CursorCollectionSuccessEvent =
  | {
      type: 'cursor_collection_request_succeeded';
      tab: 'agents';
      collection: 'agents';
      generation: number;
      items: AgentRegistryAgentSummary[];
      nextCursor: string | null;
      append: boolean;
      observedAt: string;
    }
  | {
      type: 'cursor_collection_request_succeeded';
      tab: 'tools';
      collection: 'toolDefinitions';
      generation: number;
      items: ToolRegistryToolDefinitionSummary[];
      nextCursor: string | null;
      append: boolean;
      observedAt: string;
    }
  | {
      type: 'cursor_collection_request_succeeded';
      tab: 'connectors';
      collection: 'connectorTypes' | 'connectorBindings';
      generation: number;
      items: ConnectorRegistryTypeSummary[] | ConnectorRegistryBindingSummary[];
      nextCursor: string | null;
      append: boolean;
      observedAt: string;
    };

type UnpagedCollectionSuccessEvent = {
  type: 'unpaged_collection_request_succeeded';
  tab: 'tools';
  collection: 'toolEnablements';
  generation: number;
  items: ToolRegistryToolEnablement[];
  observedAt: string;
};

type CollectionFailureEvent = {
  type: 'collection_request_failed';
  tab: RegistryTab;
  collection: RegistryCollectionKey;
  generation: number;
  error: RegistryBridgeErrorMeta;
  restartPlanned: boolean;
  failedAtMs?: number;
};

type DetailSuccessEvent =
  | {
      type: 'detail_request_succeeded';
      tab: 'agents';
      generation: number;
      detail: RegistryDetailRef;
      record: AgentRegistryAgent;
    }
  | {
      type: 'detail_request_succeeded';
      tab: 'tools';
      generation: number;
      detail: RegistryDetailRef;
      record: ToolRegistryToolDefinition | ToolRegistryToolEnablement;
    }
  | {
      type: 'detail_request_succeeded';
      tab: 'connectors';
      generation: number;
      detail: RegistryDetailRef;
      record: ConnectorRegistryType | ConnectorRegistryBinding;
    };

export type RegistryInspectionAction =
  | { type: 'tab_selected'; tab: RegistryTab; source: 'user' | 'history' | 'programmatic' }
  | { type: 'tab_request_started'; tab: RegistryTab; generation: number }
  | {
      type: 'collection_request_started';
      tab: RegistryTab;
      collection: RegistryCollectionKey;
      generation: number;
    }
  | CursorCollectionSuccessEvent
  | UnpagedCollectionSuccessEvent
  | CollectionFailureEvent
  | {
      type: 'detail_request_started';
      tab: RegistryTab;
      generation: number;
      detail: RegistryDetailRef;
    }
  | DetailSuccessEvent
  | {
      type: 'detail_request_failed';
      tab: RegistryTab;
      generation: number;
      detail: RegistryDetailRef;
      error: RegistryBridgeErrorMeta;
    }
  | { type: 'surface_blocked'; error: RegistryBridgeErrorMeta };

function notLoadedCollection<TItem>(): CursorRegistryCollectionState<TItem> {
  return {
    items: [],
    nextCursor: null,
    observation: { kind: 'not_loaded' },
    lastObservedAt: null,
    successGeneration: null,
    restartGeneration: null,
    retryDisabledUntil: null,
  };
}

function notLoadedUnpagedCollection<TItem>(): UnpagedRegistryCollectionState<TItem> {
  return {
    items: [],
    observation: { kind: 'not_loaded' },
    lastObservedAt: null,
    successGeneration: null,
    restartGeneration: null,
    retryDisabledUntil: null,
  };
}

function createAgentsTabState(): AgentsTabState {
  return {
    generation: 0,
    observedAt: null,
    summaryState: 'idle',
    detail: { kind: 'none' },
    collections: { agents: notLoadedCollection<AgentRegistryAgentSummary>() },
  };
}

function createToolsTabState(): ToolsTabState {
  return {
    generation: 0,
    observedAt: null,
    summaryState: 'idle',
    detail: { kind: 'none' },
    collections: {
      toolDefinitions: notLoadedCollection<ToolRegistryToolDefinitionSummary>(),
      toolEnablements: notLoadedUnpagedCollection<ToolRegistryToolEnablement>(),
    },
  };
}

function createConnectorsTabState(): ConnectorsTabState {
  return {
    generation: 0,
    observedAt: null,
    summaryState: 'idle',
    detail: { kind: 'none' },
    collections: {
      connectorTypes: notLoadedCollection<ConnectorRegistryTypeSummary>(),
      connectorBindings: notLoadedCollection<ConnectorRegistryBindingSummary>(),
    },
  };
}

function relevantCollectionKeys(tab: RegistryTab): RegistryCollectionKey[] {
  switch (tab) {
    case 'agents':
      return ['agents'];
    case 'tools':
      return ['toolDefinitions', 'toolEnablements'];
    case 'connectors':
      return ['connectorTypes', 'connectorBindings'];
  }
}

function getCollectionState<TTab extends RegistryTab>(
  state: RegistryInspectionState,
  tab: TTab,
  collection: RegistryCollectionKey
) {
  if (tab === 'agents' && collection === 'agents') {
    return state.tabs.agents.collections.agents;
  }

  if (tab === 'tools' && collection === 'toolDefinitions') {
    return state.tabs.tools.collections.toolDefinitions;
  }

  if (tab === 'tools' && collection === 'toolEnablements') {
    return state.tabs.tools.collections.toolEnablements;
  }

  if (tab === 'connectors' && collection === 'connectorTypes') {
    return state.tabs.connectors.collections.connectorTypes;
  }

  if (tab === 'connectors' && collection === 'connectorBindings') {
    return state.tabs.connectors.collections.connectorBindings;
  }

  return null;
}

function collectionObservationSuccess(
  observedAt: string,
  itemCount: number
): RegistryObservationState {
  return itemCount === 0
    ? { kind: 'empty', observedAt }
    : { kind: 'loaded', observedAt, stale: false };
}

function isTransientError(error: RegistryBridgeErrorMeta): boolean {
  if (error.kind === 'YouPetCoreTransport') {
    return true;
  }

  if (error.kind !== 'YouPetCoreHttpError') {
    return false;
  }

  const status = error.httpStatus;
  return status === 429 || status === undefined || status >= 500;
}

function detailsEqual(left: RegistryDetailRef | null, right: RegistryDetailRef): boolean {
  return (
    left !== null &&
    left.kind === right.kind &&
    left.key === right.key &&
    left.version === right.version
  );
}

function getRetryDisabledUntil(error: RegistryBridgeErrorMeta, failedAtMs?: number): number | null {
  if (
    error.kind !== 'YouPetCoreHttpError' ||
    error.httpStatus !== 429 ||
    typeof error.retryAfterSeconds !== 'number' ||
    !Number.isFinite(error.retryAfterSeconds) ||
    error.retryAfterSeconds <= 0
  ) {
    return null;
  }

  return (failedAtMs ?? Date.now()) + error.retryAfterSeconds * 1000;
}

function summarizeTab(state: RegistryInspectionState, tab: RegistryTab): RegistrySummaryState {
  const collections = relevantCollectionKeys(tab).map(collection =>
    getCollectionState(state, tab, collection)
  );
  if (collections.some(collection => collection?.observation.kind === 'blocked')) {
    return 'blocked';
  }

  const staleCount = collections.filter(
    collection => collection?.observation.kind === 'stale'
  ).length;
  if (staleCount > 0) {
    return collections.length === 1 ? 'stale' : 'partial';
  }

  if (collections.some(collection => collection?.observation.kind === 'loading')) {
    return 'loading';
  }

  if (
    collections.every(
      collection =>
        collection?.observation.kind === 'loaded' || collection?.observation.kind === 'empty'
    )
  ) {
    return 'fresh';
  }

  return 'idle';
}

function updateObservedAt(
  state: RegistryInspectionState,
  tab: RegistryTab,
  observedAt: string
): void {
  const tabState = state.tabs[tab];
  const complete = relevantCollectionKeys(tab).every(collection => {
    const collectionState = getCollectionState(state, tab, collection);
    return collectionState?.successGeneration === tabState.generation;
  });

  if (complete) {
    tabState.observedAt = observedAt;
  }
}

function cloneState(state: RegistryInspectionState): RegistryInspectionState {
  return {
    urlState: state.urlState,
    surfaceError: state.surfaceError,
    tabs: {
      agents: {
        ...state.tabs.agents,
        collections: {
          agents: {
            ...state.tabs.agents.collections.agents,
            items: [...state.tabs.agents.collections.agents.items],
          },
        },
      },
      tools: {
        ...state.tabs.tools,
        collections: {
          toolDefinitions: {
            ...state.tabs.tools.collections.toolDefinitions,
            items: [...state.tabs.tools.collections.toolDefinitions.items],
          },
          toolEnablements: {
            ...state.tabs.tools.collections.toolEnablements,
            items: [...state.tabs.tools.collections.toolEnablements.items],
          },
        },
      },
      connectors: {
        ...state.tabs.connectors,
        collections: {
          connectorTypes: {
            ...state.tabs.connectors.collections.connectorTypes,
            items: [...state.tabs.connectors.collections.connectorTypes.items],
          },
          connectorBindings: {
            ...state.tabs.connectors.collections.connectorBindings,
            items: [...state.tabs.connectors.collections.connectorBindings.items],
          },
        },
      },
    },
  };
}

function clearAllDetails(state: RegistryInspectionState): void {
  state.tabs.agents.detail = { kind: 'none' };
  state.tabs.tools.detail = { kind: 'none' };
  state.tabs.connectors.detail = { kind: 'none' };
}

function markTabLoading(
  state: RegistryInspectionState,
  tab: RegistryTab,
  generation: number
): void {
  const tabState = state.tabs[tab];
  if (generation < tabState.generation) {
    return;
  }

  tabState.generation = generation;
  tabState.summaryState = 'loading';

  for (const collection of relevantCollectionKeys(tab)) {
    const collectionState = getCollectionState(state, tab, collection);
    if (!collectionState) {
      continue;
    }
    collectionState.observation = { kind: 'loading', generation };
  }

  if (state.urlState.tab === tab && state.urlState.detail) {
    tabState.detail = { kind: 'loading', detail: state.urlState.detail, generation };
  }
}

function markCollectionLoading(
  state: RegistryInspectionState,
  tab: RegistryTab,
  collection: RegistryCollectionKey,
  generation: number
): void {
  const tabState = state.tabs[tab];
  if (generation < tabState.generation) {
    return;
  }

  const collectionState = getCollectionState(state, tab, collection);
  if (!collectionState) {
    return;
  }

  tabState.generation = generation;
  collectionState.observation = { kind: 'loading', generation };
}

function resetCollectionForRestart(
  collectionState: CursorRegistryCollectionState<unknown> | UnpagedRegistryCollectionState<unknown>,
  generation: number
): void {
  collectionState.items = [];
  if ('nextCursor' in collectionState) {
    collectionState.nextCursor = null;
  }
  collectionState.observation = { kind: 'loading', generation };
  collectionState.restartGeneration = generation;
  collectionState.successGeneration = null;
  collectionState.retryDisabledUntil = null;
}

function blockCollection(
  collectionState: CursorRegistryCollectionState<unknown> | UnpagedRegistryCollectionState<unknown>,
  error: RegistryBridgeErrorMeta,
  retryDisabledUntil: number | null
): void {
  collectionState.items = [];
  if ('nextCursor' in collectionState) {
    collectionState.nextCursor = null;
  }
  collectionState.observation = { kind: 'blocked', error };
  collectionState.successGeneration = null;
  collectionState.retryDisabledUntil = retryDisabledUntil;
}

export function createRegistryInspectionState(
  urlState: RegistryUrlState = { tab: 'agents', detail: null }
): RegistryInspectionState {
  return {
    urlState,
    surfaceError: null,
    tabs: {
      agents: createAgentsTabState(),
      tools: createToolsTabState(),
      connectors: createConnectorsTabState(),
    },
  };
}

export function registryInspectionReducer(
  state: RegistryInspectionState,
  action: RegistryInspectionAction
): RegistryInspectionState {
  const next = cloneState(state);

  switch (action.type) {
    case 'tab_selected': {
      next.urlState = { tab: action.tab, detail: null };
      clearAllDetails(next);
      next.tabs[action.tab].summaryState = summarizeTab(next, action.tab);
      return next;
    }

    case 'tab_request_started': {
      next.surfaceError = null;
      markTabLoading(next, action.tab, action.generation);
      return next;
    }

    case 'collection_request_started': {
      next.surfaceError = null;
      markCollectionLoading(next, action.tab, action.collection, action.generation);
      next.tabs[action.tab].summaryState = summarizeTab(next, action.tab);
      return next;
    }

    case 'cursor_collection_request_succeeded': {
      const tabState = next.tabs[action.tab];
      if (action.generation !== tabState.generation) {
        return state;
      }

      const collectionState = getCollectionState(next, action.tab, action.collection);
      if (!collectionState || !('nextCursor' in collectionState)) {
        return state;
      }

      const items = action.append
        ? [...(collectionState.items as Array<(typeof action.items)[number]>), ...action.items]
        : [...action.items];
      collectionState.items = items as typeof collectionState.items;
      collectionState.nextCursor = action.nextCursor;
      collectionState.lastObservedAt = action.observedAt;
      collectionState.successGeneration = action.generation;
      collectionState.observation = collectionObservationSuccess(action.observedAt, items.length);
      collectionState.retryDisabledUntil = null;
      updateObservedAt(next, action.tab, action.observedAt);
      tabState.summaryState = summarizeTab(next, action.tab);
      return next;
    }

    case 'unpaged_collection_request_succeeded': {
      const tabState = next.tabs[action.tab];
      if (action.generation !== tabState.generation) {
        return state;
      }

      const collectionState = getCollectionState(next, action.tab, action.collection);
      if (!collectionState || 'nextCursor' in collectionState) {
        return state;
      }

      collectionState.items = [...action.items];
      collectionState.lastObservedAt = action.observedAt;
      collectionState.successGeneration = action.generation;
      collectionState.observation = collectionObservationSuccess(
        action.observedAt,
        action.items.length
      );
      collectionState.retryDisabledUntil = null;
      updateObservedAt(next, action.tab, action.observedAt);
      tabState.summaryState = summarizeTab(next, action.tab);
      return next;
    }

    case 'collection_request_failed': {
      const tabState = next.tabs[action.tab];
      if (action.generation !== tabState.generation) {
        return state;
      }

      const collectionState = getCollectionState(next, action.tab, action.collection);
      if (!collectionState) {
        return state;
      }

      const retryDisabledUntil = getRetryDisabledUntil(action.error, action.failedAtMs);

      if (action.restartPlanned) {
        resetCollectionForRestart(collectionState, action.generation);
      } else if (
        collectionState.items.length > 0 &&
        isTransientError(action.error) &&
        collectionState.lastObservedAt
      ) {
        collectionState.observation = {
          kind: 'stale',
          observedAt: collectionState.lastObservedAt,
          error: action.error,
        };
        collectionState.retryDisabledUntil = retryDisabledUntil;
      } else {
        blockCollection(collectionState, action.error, retryDisabledUntil);
      }

      tabState.summaryState = summarizeTab(next, action.tab);
      return next;
    }

    case 'detail_request_started': {
      const tabState = next.tabs[action.tab];
      if (action.generation !== tabState.generation) {
        return state;
      }

      next.urlState = { tab: action.tab, detail: action.detail };
      next.surfaceError = null;
      tabState.detail = { kind: 'loading', detail: action.detail, generation: action.generation };
      return next;
    }

    case 'detail_request_succeeded': {
      const tabState = next.tabs[action.tab];
      if (
        action.generation !== tabState.generation ||
        next.urlState.tab !== action.tab ||
        !detailsEqual(next.urlState.detail, action.detail)
      ) {
        return state;
      }

      tabState.detail = { kind: 'loaded', detail: action.detail, record: action.record };
      return next;
    }

    case 'detail_request_failed': {
      const tabState = next.tabs[action.tab];
      if (
        action.generation !== tabState.generation ||
        next.urlState.tab !== action.tab ||
        !detailsEqual(next.urlState.detail, action.detail)
      ) {
        return state;
      }

      tabState.detail =
        action.error.kind === 'YouPetCoreHttpError' && action.error.httpStatus === 404
          ? { kind: 'missing', detail: action.detail, error: action.error }
          : { kind: 'error', detail: action.detail, error: action.error };
      return next;
    }

    case 'surface_blocked': {
      next.surfaceError = action.error;
      next.urlState = { tab: next.urlState.tab, detail: null };
      next.tabs.agents = createAgentsTabState();
      next.tabs.tools = createToolsTabState();
      next.tabs.connectors = createConnectorsTabState();
      next.tabs.agents.summaryState = 'blocked';
      next.tabs.tools.summaryState = 'blocked';
      next.tabs.connectors.summaryState = 'blocked';
      return next;
    }
  }
}
