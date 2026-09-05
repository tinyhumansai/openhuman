import { useCallback, useEffect, useRef, useState } from 'react';

import {
  type AgentRegistryAgent,
  type AgentRegistryAgentSummary,
  type ConnectorRegistryBinding,
  type ConnectorRegistryBindingSummary,
  type ConnectorRegistryType,
  type ConnectorRegistryTypeSummary,
  coreRegistriesClient,
  type CursorRegistryPage,
  extractRegistryBridgeErrorMeta,
  type RegistryCursorListParams,
  type ToolRegistryToolDefinition,
  type ToolRegistryToolDefinitionSummary,
  type ToolRegistryToolEnablement,
  type UnpagedRegistryCollection,
} from '../../services/api/coreRegistriesClient';
import { createRegistryInspectionState, LOAD_MORE_LIMIT, registryInspectionReducer } from './state';
import {
  getRegistryTabForDetailKind,
  type RegistryCollectionKey,
  type RegistryDetailRecord,
  type RegistryDetailRef,
  type RegistryInspectionState,
  type RegistryTab,
} from './types';
import { parseRegistryUrlState, serializeRegistryUrlState } from './urlState';

export interface RegistryInspectionClient {
  listAgents: (
    params?: RegistryCursorListParams
  ) => Promise<CursorRegistryPage<AgentRegistryAgentSummary>>;
  getAgentVersion: (params: { agentKey: string; version: number }) => Promise<AgentRegistryAgent>;
  listToolDefinitions: (
    params?: RegistryCursorListParams
  ) => Promise<CursorRegistryPage<ToolRegistryToolDefinitionSummary>>;
  getToolDefinitionVersion: (params: {
    toolKey: string;
    version: number;
  }) => Promise<ToolRegistryToolDefinition>;
  listToolEnablements: () => Promise<UnpagedRegistryCollection<ToolRegistryToolEnablement>>;
  getToolEnablementVersion: (params: {
    toolKey: string;
    version: number;
  }) => Promise<ToolRegistryToolEnablement>;
  listConnectorTypes: (
    params?: RegistryCursorListParams
  ) => Promise<CursorRegistryPage<ConnectorRegistryTypeSummary>>;
  getConnectorTypeVersion: (params: {
    connectorKey: string;
    version: number;
  }) => Promise<ConnectorRegistryType>;
  listConnectorBindings: (
    params?: RegistryCursorListParams
  ) => Promise<CursorRegistryPage<ConnectorRegistryBindingSummary>>;
  getConnectorBindingVersion: (params: {
    bindingKey: string;
    version: number;
  }) => Promise<ConnectorRegistryBinding>;
}

interface UseRegistryInspectionOptions {
  client?: RegistryInspectionClient;
}

export interface UseRegistryInspectionResult {
  state: RegistryInspectionState;
  setTab: (tab: RegistryTab) => Promise<void>;
  refreshActiveTab: () => Promise<void>;
  loadMoreCollection: (collection: RegistryCollectionKey) => Promise<void>;
  openDetail: (detail: RegistryDetailRef) => Promise<void>;
  retryCollection: (collection: RegistryCollectionKey) => Promise<void>;
}

type ExactCache = Map<string, RegistryDetailRecord>;

function cacheKey(detail: RegistryDetailRef): string {
  return `${detail.kind}:${detail.key}:${detail.version}`;
}

function isPageSessionDetailCacheable(detail: RegistryDetailRef): boolean {
  return detail.kind !== 'tool-enablement';
}

function currentObservedAt(): string {
  return new Date().toISOString();
}

function isSurfaceBlockingError(error: ReturnType<typeof extractRegistryBridgeErrorMeta>) {
  if (!error) {
    return false;
  }

  if (error.kind === 'YouPetConfigMissing' || error.kind === 'YouPetConfigInvalid') {
    return true;
  }

  if (error.kind !== 'YouPetCoreHttpError') {
    return false;
  }

  return (
    error.httpStatus === 401 ||
    error.httpStatus === 403 ||
    (error.httpStatus === 503 &&
      (error.coreCode === 'kernel_tenant_unavailable' ||
        error.coreCode === 'kernel_tenant_invariant_violation'))
  );
}

function isInvalidCursorError(error: ReturnType<typeof extractRegistryBridgeErrorMeta>) {
  return Boolean(
    error &&
    error.kind === 'YouPetCoreHttpError' &&
    error.httpStatus === 422 &&
    error.coreCode === 'invalid_cursor'
  );
}

function pushUrlState(
  urlState: RegistryInspectionState['urlState'],
  mode: 'push' | 'replace'
): void {
  const serialized = serializeRegistryUrlState(urlState);
  const search = serialized.length > 0 ? `?${serialized}` : '';
  const nextUrl = `${window.location.pathname}${search}`;

  if (mode === 'replace') {
    window.history.replaceState({}, '', nextUrl);
    return;
  }

  window.history.pushState({}, '', nextUrl);
}

function collectionTab(collection: RegistryCollectionKey): RegistryTab {
  switch (collection) {
    case 'agents':
      return 'agents';
    case 'toolDefinitions':
    case 'toolEnablements':
      return 'tools';
    case 'connectorTypes':
    case 'connectorBindings':
      return 'connectors';
  }
}

function relevantCollectionKeysForTab(tab: RegistryTab): RegistryCollectionKey[] {
  switch (tab) {
    case 'agents':
      return ['agents'];
    case 'tools':
      return ['toolDefinitions', 'toolEnablements'];
    case 'connectors':
      return ['connectorTypes', 'connectorBindings'];
  }
}

function browserSelectsDetail(tab: RegistryTab, detail: RegistryDetailRef): boolean {
  const current = parseRegistryUrlState(window.location.search);
  return (
    current.tab === tab &&
    current.detail?.kind === detail.kind &&
    current.detail.key === detail.key &&
    current.detail.version === detail.version
  );
}

function getCollectionState(
  state: RegistryInspectionState,
  tab: RegistryTab,
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

function isRetryCooldownActive(retryDisabledUntil: number | null | undefined): boolean {
  return typeof retryDisabledUntil === 'number' && retryDisabledUntil > Date.now();
}

function hasInFlightCollection(state: RegistryInspectionState, tab: RegistryTab): boolean {
  return relevantCollectionKeysForTab(tab).some(collection => {
    const collectionState = getCollectionState(state, tab, collection);
    return collectionState?.observation.kind === 'loading';
  });
}

export function useRegistryInspection(
  options: UseRegistryInspectionOptions = {}
): UseRegistryInspectionResult {
  const client = options.client ?? coreRegistriesClient;
  const [state, setState] = useState<RegistryInspectionState>(() =>
    createRegistryInspectionState(parseRegistryUrlState(window.location.search))
  );
  const stateRef = useRef(state);
  const visitedTabsRef = useRef(new Set<RegistryTab>());
  const detailCacheRef = useRef<ExactCache>(new Map());
  const generationRef = useRef<Record<RegistryTab, number>>({ agents: 0, tools: 0, connectors: 0 });

  useEffect(() => {
    stateRef.current = state;
  }, [state]);

  useEffect(() => {
    if (!state.surfaceError) {
      return;
    }

    visitedTabsRef.current.clear();

    const serialized = serializeRegistryUrlState(state.urlState);
    const nextSearch = serialized.length > 0 ? `?${serialized}` : '';
    if (window.location.search !== nextSearch) {
      pushUrlState(state.urlState, 'replace');
    }
  }, [state.surfaceError, state.urlState]);

  const dispatch = useCallback((action: Parameters<typeof registryInspectionReducer>[1]) => {
    setState(current => {
      const next = registryInspectionReducer(current, action);
      stateRef.current = next;
      return next;
    });
  }, []);

  const nextGeneration = useCallback((tab: RegistryTab) => {
    generationRef.current[tab] += 1;
    return generationRef.current[tab];
  }, []);

  const runDetailRequest = useCallback(
    async (tab: RegistryTab, detail: RegistryDetailRef, generation: number) => {
      const detailIsCacheable = isPageSessionDetailCacheable(detail);
      const cached = detailIsCacheable ? detailCacheRef.current.get(cacheKey(detail)) : undefined;
      if (cached) {
        dispatch({ type: 'detail_request_started', tab, generation, detail });
        dispatch({
          type: 'detail_request_succeeded',
          tab,
          generation,
          detail,
          record: cached as never,
        });
        return;
      }

      dispatch({ type: 'detail_request_started', tab, generation, detail });

      try {
        let record: RegistryDetailRecord;
        switch (detail.kind) {
          case 'agent':
            record = await client.getAgentVersion({
              agentKey: detail.key,
              version: detail.version,
            });
            break;
          case 'tool-definition':
            record = await client.getToolDefinitionVersion({
              toolKey: detail.key,
              version: detail.version,
            });
            break;
          case 'tool-enablement':
            record = await client.getToolEnablementVersion({
              toolKey: detail.key,
              version: detail.version,
            });
            break;
          case 'connector-type':
            record = await client.getConnectorTypeVersion({
              connectorKey: detail.key,
              version: detail.version,
            });
            break;
          case 'connector-binding':
            record = await client.getConnectorBindingVersion({
              bindingKey: detail.key,
              version: detail.version,
            });
            break;
        }

        if (detailIsCacheable) {
          detailCacheRef.current.set(cacheKey(detail), record);
        } else {
          detailCacheRef.current.delete(cacheKey(detail));
        }
        dispatch({
          type: 'detail_request_succeeded',
          tab,
          generation,
          detail,
          record: record as never,
        });
      } catch (error) {
        const meta = extractRegistryBridgeErrorMeta(error);
        if (isSurfaceBlockingError(meta) && meta) {
          dispatch({ type: 'surface_blocked', error: meta });
          return;
        }

        dispatch({
          type: 'detail_request_failed',
          tab,
          generation,
          detail,
          error: meta ?? { kind: 'YouPetCoreTransport' },
        });
      }
    },
    [client, dispatch]
  );

  const runCollectionRequest = useCallback(
    async (
      tab: RegistryTab,
      collection: RegistryCollectionKey,
      generation: number,
      options: { append: boolean; cursor?: string; restarted?: boolean } = { append: false }
    ) => {
      try {
        const observedAt = currentObservedAt();

        switch (collection) {
          case 'agents': {
            const response = await client.listAgents({
              limit: LOAD_MORE_LIMIT,
              ...(options.cursor ? { cursor: options.cursor } : {}),
            });
            dispatch({
              type: 'cursor_collection_request_succeeded',
              tab: 'agents',
              collection: 'agents',
              generation,
              items: response.items,
              nextCursor: response.nextCursor,
              append: options.append,
              observedAt,
            });
            return;
          }

          case 'toolDefinitions': {
            const response = await client.listToolDefinitions({
              limit: LOAD_MORE_LIMIT,
              ...(options.cursor ? { cursor: options.cursor } : {}),
            });
            dispatch({
              type: 'cursor_collection_request_succeeded',
              tab: 'tools',
              collection: 'toolDefinitions',
              generation,
              items: response.items,
              nextCursor: response.nextCursor,
              append: options.append,
              observedAt,
            });
            return;
          }

          case 'toolEnablements': {
            const response = await client.listToolEnablements();
            dispatch({
              type: 'unpaged_collection_request_succeeded',
              tab: 'tools',
              collection: 'toolEnablements',
              generation,
              items: response.items,
              observedAt,
            });
            return;
          }

          case 'connectorTypes': {
            const response = await client.listConnectorTypes({
              limit: LOAD_MORE_LIMIT,
              ...(options.cursor ? { cursor: options.cursor } : {}),
            });
            dispatch({
              type: 'cursor_collection_request_succeeded',
              tab: 'connectors',
              collection: 'connectorTypes',
              generation,
              items: response.items,
              nextCursor: response.nextCursor,
              append: options.append,
              observedAt,
            });
            return;
          }

          case 'connectorBindings': {
            const response = await client.listConnectorBindings({
              limit: LOAD_MORE_LIMIT,
              ...(options.cursor ? { cursor: options.cursor } : {}),
            });
            dispatch({
              type: 'cursor_collection_request_succeeded',
              tab: 'connectors',
              collection: 'connectorBindings',
              generation,
              items: response.items,
              nextCursor: response.nextCursor,
              append: options.append,
              observedAt,
            });
            return;
          }
        }
      } catch (error) {
        const meta = extractRegistryBridgeErrorMeta(error) ?? {
          kind: 'YouPetCoreTransport' as const,
        };
        if (isSurfaceBlockingError(meta)) {
          dispatch({ type: 'surface_blocked', error: meta });
          return;
        }

        const collectionState = getCollectionState(stateRef.current, tab, collection);
        const shouldRestart = Boolean(
          options.cursor &&
          !options.restarted &&
          isInvalidCursorError(meta) &&
          collectionState?.restartGeneration !== generation
        );
        dispatch({
          type: 'collection_request_failed',
          tab,
          collection,
          generation,
          error: meta,
          restartPlanned: shouldRestart,
          failedAtMs: Date.now(),
        });

        if (shouldRestart) {
          await runCollectionRequest(tab, collection, generation, {
            append: false,
            restarted: true,
          });
        }
      }
    },
    [client, dispatch]
  );

  const loadTabGeneration = useCallback(
    async (tab: RegistryTab, generation: number) => {
      dispatch({ type: 'tab_request_started', tab, generation });

      if (tab === 'agents') {
        await runCollectionRequest('agents', 'agents', generation, { append: false });
      } else if (tab === 'tools') {
        await Promise.all([
          runCollectionRequest('tools', 'toolDefinitions', generation, { append: false }),
          runCollectionRequest('tools', 'toolEnablements', generation, { append: false }),
        ]);
      } else {
        await Promise.all([
          runCollectionRequest('connectors', 'connectorTypes', generation, { append: false }),
          runCollectionRequest('connectors', 'connectorBindings', generation, { append: false }),
        ]);
      }
    },
    [dispatch, runCollectionRequest]
  );

  const ensureTabLoaded = useCallback(
    async (tab: RegistryTab) => {
      if (stateRef.current.surfaceError) {
        visitedTabsRef.current.delete(tab);
      }
      if (visitedTabsRef.current.has(tab)) {
        return;
      }

      visitedTabsRef.current.add(tab);
      const generation = nextGeneration(tab);
      await loadTabGeneration(tab, generation);
    },
    [loadTabGeneration, nextGeneration]
  );

  const setTab = useCallback(
    async (tab: RegistryTab) => {
      dispatch({ type: 'tab_selected', tab, source: 'user' });
      pushUrlState({ tab, detail: null }, 'push');
      await ensureTabLoaded(tab);
    },
    [dispatch, ensureTabLoaded]
  );

  const refreshActiveTab = useCallback(async () => {
    const tab = stateRef.current.urlState.tab;
    for (const collection of relevantCollectionKeysForTab(tab)) {
      const collectionState = getCollectionState(stateRef.current, tab, collection);
      if (isRetryCooldownActive(collectionState?.retryDisabledUntil)) {
        return;
      }
    }

    visitedTabsRef.current.add(tab);
    const generation = nextGeneration(tab);
    await loadTabGeneration(tab, generation);
    const selected = stateRef.current.urlState;
    if (selected.tab === tab && selected.detail) {
      await runDetailRequest(tab, selected.detail, generation);
    }
  }, [loadTabGeneration, nextGeneration, runDetailRequest]);

  const loadMoreCollection = useCallback(
    async (collection: RegistryCollectionKey) => {
      const tab = collectionTab(collection);
      const current = stateRef.current;
      const collectionState =
        tab === 'agents'
          ? current.tabs.agents.collections.agents
          : tab === 'tools'
            ? collection === 'toolDefinitions'
              ? current.tabs.tools.collections.toolDefinitions
              : current.tabs.tools.collections.toolEnablements
            : collection === 'connectorTypes'
              ? current.tabs.connectors.collections.connectorTypes
              : current.tabs.connectors.collections.connectorBindings;

      if (!('nextCursor' in collectionState) || !collectionState.nextCursor) {
        return;
      }

      if (isRetryCooldownActive(collectionState.retryDisabledUntil)) {
        return;
      }

      await runCollectionRequest(tab, collection, current.tabs[tab].generation, {
        append: true,
        cursor: collectionState.nextCursor,
      });
    },
    [runCollectionRequest]
  );

  const openDetail = useCallback(
    async (detail: RegistryDetailRef) => {
      const tab = getRegistryTabForDetailKind(detail.kind);
      if (stateRef.current.urlState.tab !== tab) {
        dispatch({ type: 'tab_selected', tab, source: 'programmatic' });
        pushUrlState({ tab, detail }, 'push');
        await ensureTabLoaded(tab);
      } else {
        pushUrlState({ tab, detail }, 'push');
      }

      if (stateRef.current.surfaceError || !browserSelectsDetail(tab, detail)) {
        return;
      }

      const generation = stateRef.current.tabs[tab].generation;
      await runDetailRequest(tab, detail, generation);
    },
    [dispatch, ensureTabLoaded, runDetailRequest]
  );

  const retryCollection = useCallback(
    async (collection: RegistryCollectionKey) => {
      const tab = collectionTab(collection);
      const collectionState = getCollectionState(stateRef.current, tab, collection);
      if (isRetryCooldownActive(collectionState?.retryDisabledUntil)) {
        return;
      }
      if (hasInFlightCollection(stateRef.current, tab)) {
        return;
      }

      visitedTabsRef.current.add(tab);
      const generation = nextGeneration(tab);
      dispatch({ type: 'collection_request_started', tab, collection, generation });
      await runCollectionRequest(tab, collection, generation, { append: false });
    },
    [dispatch, nextGeneration, runCollectionRequest]
  );

  useEffect(() => {
    const initialUrlState = parseRegistryUrlState(window.location.search);
    const canonical = serializeRegistryUrlState(initialUrlState);
    if (!(window.location.search === '' && canonical === 'tab=agents')) {
      const normalizedSearch = canonical ? `?${canonical}` : '';
      if (window.location.search !== normalizedSearch) {
        pushUrlState(initialUrlState, 'replace');
      }
    }

    void ensureTabLoaded(initialUrlState.tab).then(async () => {
      if (
        initialUrlState.detail &&
        !stateRef.current.surfaceError &&
        browserSelectsDetail(initialUrlState.tab, initialUrlState.detail)
      ) {
        await runDetailRequest(
          initialUrlState.tab,
          initialUrlState.detail,
          stateRef.current.tabs[initialUrlState.tab].generation
        );
      }
    });

    const onPopState = () => {
      const nextUrlState = parseRegistryUrlState(window.location.search);
      dispatch({ type: 'tab_selected', tab: nextUrlState.tab, source: 'history' });

      void ensureTabLoaded(nextUrlState.tab).then(async () => {
        if (
          !nextUrlState.detail ||
          stateRef.current.surfaceError ||
          !browserSelectsDetail(nextUrlState.tab, nextUrlState.detail)
        ) {
          return;
        }

        const cached = isPageSessionDetailCacheable(nextUrlState.detail)
          ? detailCacheRef.current.get(cacheKey(nextUrlState.detail))
          : undefined;
        const generation = stateRef.current.tabs[nextUrlState.tab].generation;
        if (cached) {
          dispatch({
            type: 'detail_request_started',
            tab: nextUrlState.tab,
            generation,
            detail: nextUrlState.detail,
          });
          dispatch({
            type: 'detail_request_succeeded',
            tab: nextUrlState.tab,
            generation,
            detail: nextUrlState.detail,
            record: cached as never,
          });
          return;
        }

        await runDetailRequest(nextUrlState.tab, nextUrlState.detail, generation);
      });
    };

    window.addEventListener('popstate', onPopState);
    return () => window.removeEventListener('popstate', onPopState);
  }, [dispatch, ensureTabLoaded, runDetailRequest]);

  return { state, setTab, refreshActiveTab, loadMoreCollection, openDetail, retryCollection };
}
