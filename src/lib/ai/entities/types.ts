/** Entity types in the platform graph */
export type EntityType =
  | 'contact'
  | 'chat'
  | 'message'
  | 'email'
  | 'wallet'
  | 'token'
  | 'transaction';

/** Source system for an entity */
export type EntitySource = 'telegram' | 'gmail' | 'manual' | 'onchain';

/** Relationship types between entities */
export type RelationType = 'member_of' | 'sent_by' | 'sent_to' | 'owns' | 'traded' | 'replied_to';

/** Core entity record (frontend domain type) */
export interface Entity {
  id: string;
  type: EntityType;
  source: EntitySource;
  sourceId: string | null;
  title: string | null;
  summary: string | null;
  /** JSON blob for type-specific fields */
  metadata: string | null;
  createdAt: number;
  updatedAt: number;
}

/** Relationship between two entities */
export interface EntityRelation {
  id: string;
  fromEntityId: string;
  toEntityId: string;
  relationType: RelationType;
  metadata: string | null;
  createdAt: number;
}

/** Tag attached to an entity */
export interface EntityTag {
  entityId: string;
  tag: string;
}

/** Search result from entity search */
export interface EntitySearchResult extends Entity {
  score: number;
}

// --- Neo4j backend response types ---

/** Entity node as returned from the Neo4j backend API */
export interface Neo4jEntityNode {
  id: string;
  name: string;
  type: string;
  description: string | null;
  properties: string | null;
  confidence: number;
  isActive: boolean;
  ownerId: string;
  createdAt: string;
  updatedAt: string;
}

/** Relationship record as returned from the Neo4j backend API */
export interface Neo4jRelationshipRecord {
  id: string;
  sourceId: string;
  targetId: string;
  type: string;
  properties: string | null;
  weight: number;
  createdAt: string;
}

/** Wrapper for all API responses */
export interface ApiResponse<T> {
  success: boolean;
  data: T;
}

// --- Conversion helpers ---

/** Parse a properties JSON string, returning an empty object on failure */
function parseProperties(props: string | null): Record<string, unknown> {
  if (!props) return {};
  try {
    return JSON.parse(props);
  } catch {
    return {};
  }
}

/** Convert a Neo4j entity node to the frontend Entity type */
export function fromNeo4jEntity(node: Neo4jEntityNode): Entity {
  const props = parseProperties(node.properties);

  const source = (props.source as string) ?? 'manual';
  const sourceId = (props.sourceId as string) ?? null;

  // Build metadata from remaining properties (excluding source, sourceId, tags)
  const { source: _s, sourceId: _sid, tags: _t, ...rest } = props;
  const metadata = Object.keys(rest).length > 0 ? JSON.stringify(rest) : null;

  return {
    id: node.id,
    type: node.type as EntityType,
    source: source as EntitySource,
    sourceId,
    title: node.name,
    summary: node.description,
    metadata,
    createdAt: new Date(node.createdAt).getTime(),
    updatedAt: new Date(node.updatedAt).getTime(),
  };
}

/** Build a request body for POST/PUT to the Neo4j entity API */
export function toNeo4jCreateBody(entity: Entity): Record<string, unknown> {
  // Merge source, sourceId, tags, and any existing metadata into properties
  const existingMeta = parseProperties(entity.metadata);
  const properties: Record<string, unknown> = { ...existingMeta, source: entity.source };
  if (entity.sourceId) {
    properties.sourceId = entity.sourceId;
  }

  return {
    name: entity.title ?? '',
    type: entity.type,
    description: entity.summary ?? undefined,
    properties: JSON.stringify(properties),
    confidence: 1.0,
  };
}

/** Convert a Neo4j relationship record to the frontend EntityRelation type */
export function fromNeo4jRelation(r: Neo4jRelationshipRecord): EntityRelation {
  return {
    id: r.id,
    fromEntityId: r.sourceId,
    toEntityId: r.targetId,
    relationType: r.type as RelationType,
    metadata: r.properties,
    createdAt: new Date(r.createdAt).getTime(),
  };
}

/** Build a request body for POST to the Neo4j relationship API */
export function toNeo4jRelationBody(relation: EntityRelation): Record<string, unknown> {
  return {
    sourceId: relation.fromEntityId,
    targetId: relation.toEntityId,
    type: relation.relationType,
    properties: relation.metadata ?? undefined,
  };
}

// --- Metadata interfaces ---

/** Contact metadata */
export interface ContactMetadata {
  username?: string;
  phone?: string;
  bio?: string;
}

/** Chat metadata */
export interface ChatMetadata {
  chatType?: 'group' | 'channel' | 'private';
  memberCount?: number;
  isChannel?: boolean;
}

/** Message metadata */
export interface MessageMetadata {
  chatId?: string;
  replyToId?: string;
  mediaType?: string;
}

/** Email metadata */
export interface EmailMetadata {
  threadId?: string;
  labels?: string[];
  hasAttachments?: boolean;
}

/** Wallet metadata */
export interface WalletMetadata {
  chain?: string;
  address?: string;
  label?: string;
}

/** Token metadata */
export interface TokenMetadata {
  chain?: string;
  contract?: string;
  symbol?: string;
  decimals?: number;
}

/** Transaction metadata */
export interface TransactionMetadata {
  chain?: string;
  txHash?: string;
  value?: string;
  method?: string;
}
