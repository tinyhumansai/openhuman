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

export const REGISTRY_TABS = ['agents', 'tools', 'connectors'] as const;
export const REGISTRY_DETAIL_KINDS = [
  'agent',
  'tool-definition',
  'tool-enablement',
  'connector-type',
  'connector-binding',
] as const;
export const REGISTRY_KEY_MAX_LENGTH = 128;

export type RegistryTab = (typeof REGISTRY_TABS)[number];
export type RegistryDetailKind = (typeof REGISTRY_DETAIL_KINDS)[number];
export type RegistrySummaryState = 'idle' | 'loading' | 'fresh' | 'stale' | 'partial' | 'blocked';

export interface RegistryDetailRef {
  kind: RegistryDetailKind;
  key: string;
  version: number;
}

export interface RegistryUrlState {
  tab: RegistryTab;
  detail: RegistryDetailRef | null;
}

export type RegistryObservationState =
  | { kind: 'not_loaded' }
  | { kind: 'loading'; generation: number }
  | { kind: 'empty'; observedAt: string }
  | { kind: 'loaded'; observedAt: string; stale: false }
  | { kind: 'stale'; observedAt: string; error: RegistryBridgeErrorMeta }
  | { kind: 'blocked'; error: RegistryBridgeErrorMeta };

export type RegistryDetailRecord =
  | AgentRegistryAgent
  | ToolRegistryToolDefinition
  | ToolRegistryToolEnablement
  | ConnectorRegistryType
  | ConnectorRegistryBinding;

export type RegistryDetailState =
  | { kind: 'none' }
  | { kind: 'loading'; detail: RegistryDetailRef; generation: number }
  | { kind: 'loaded'; detail: RegistryDetailRef; record: RegistryDetailRecord }
  | { kind: 'missing'; detail: RegistryDetailRef; error: RegistryBridgeErrorMeta }
  | { kind: 'error'; detail: RegistryDetailRef; error: RegistryBridgeErrorMeta };

export interface RegistryCollectionStateBase<TItem> {
  items: TItem[];
  observation: RegistryObservationState;
  lastObservedAt: string | null;
  successGeneration: number | null;
  restartGeneration: number | null;
  retryDisabledUntil?: number | null;
}

export interface CursorRegistryCollectionState<TItem> extends RegistryCollectionStateBase<TItem> {
  nextCursor: string | null;
}

export interface UnpagedRegistryCollectionState<TItem> extends RegistryCollectionStateBase<TItem> {}

export interface AgentsTabCollections {
  agents: CursorRegistryCollectionState<AgentRegistryAgentSummary>;
}

export interface ToolsTabCollections {
  toolDefinitions: CursorRegistryCollectionState<ToolRegistryToolDefinitionSummary>;
  toolEnablements: UnpagedRegistryCollectionState<ToolRegistryToolEnablement>;
}

export interface ConnectorsTabCollections {
  connectorTypes: CursorRegistryCollectionState<ConnectorRegistryTypeSummary>;
  connectorBindings: CursorRegistryCollectionState<ConnectorRegistryBindingSummary>;
}

export interface RegistryTabState<TCollections> {
  generation: number;
  observedAt: string | null;
  summaryState: RegistrySummaryState;
  detail: RegistryDetailState;
  collections: TCollections;
}

export interface RegistryInspectionState {
  urlState: RegistryUrlState;
  surfaceError: RegistryBridgeErrorMeta | null;
  tabs: {
    agents: RegistryTabState<AgentsTabCollections>;
    tools: RegistryTabState<ToolsTabCollections>;
    connectors: RegistryTabState<ConnectorsTabCollections>;
  };
}

export type CursorCollectionKey =
  | keyof AgentsTabCollections
  | keyof Pick<ToolsTabCollections, 'toolDefinitions'>
  | keyof ConnectorsTabCollections;

export type UnpagedCollectionKey = keyof Pick<ToolsTabCollections, 'toolEnablements'>;
export type RegistryCollectionKey = CursorCollectionKey | UnpagedCollectionKey;

const ALLOWED_KINDS_BY_TAB: Record<RegistryTab, readonly RegistryDetailKind[]> = {
  agents: ['agent'],
  tools: ['tool-definition', 'tool-enablement'],
  connectors: ['connector-type', 'connector-binding'],
};

const TAB_BY_KIND: Record<RegistryDetailKind, RegistryTab> = {
  agent: 'agents',
  'tool-definition': 'tools',
  'tool-enablement': 'tools',
  'connector-type': 'connectors',
  'connector-binding': 'connectors',
};

export function isRegistryTab(value: string): value is RegistryTab {
  return REGISTRY_TABS.includes(value as RegistryTab);
}

export function isRegistryDetailKind(value: string): value is RegistryDetailKind {
  return REGISTRY_DETAIL_KINDS.includes(value as RegistryDetailKind);
}

export function getRegistryTabForDetailKind(kind: RegistryDetailKind): RegistryTab {
  return TAB_BY_KIND[kind];
}

export function isDetailAllowedForTab(tab: RegistryTab, kind: RegistryDetailKind): boolean {
  return ALLOWED_KINDS_BY_TAB[tab].includes(kind);
}
