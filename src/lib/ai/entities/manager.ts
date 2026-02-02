import { apiClient } from '../../../services/apiClient';
import {
  type ApiResponse,
  type Entity,
  type EntityRelation,
  type EntitySearchResult,
  type EntitySource,
  type EntityType,
  fromNeo4jEntity,
  fromNeo4jRelation,
  type Neo4jEntityNode,
  type Neo4jRelationshipRecord,
  type RelationType,
  toNeo4jCreateBody,
  toNeo4jRelationBody,
} from './types';

const ENTITY_API = '/api/entity-graph';
const MAX_TAG_RETRIES = 3;

/** Parse the properties JSON from a Neo4j entity node */
function parseProps(props: string | null): Record<string, unknown> {
  if (!props) return {};
  try {
    return JSON.parse(props);
  } catch {
    return {};
  }
}

/** Check whether an error represents an HTTP 409 Conflict */
function isConflictError(err: unknown): boolean {
  if (!err || typeof err !== 'object') return false;
  const error = (err as Record<string, unknown>).error;
  return typeof error === 'string' && error.includes('409');
}

/**
 * EntityManager wraps REST API calls to the Neo4j-backed entity graph.
 *
 * Provides a typed interface for CRUD operations on entities, relations,
 * and tags in the platform graph via the backend API.
 */
export class EntityManager {
  private initialized = false;

  /** Initialize the entity manager (no-op for remote backend) */
  async init(): Promise<void> {
    this.initialized = true;
  }

  /** Ensure the manager is initialized */
  private async ensureInit(): Promise<void> {
    if (!this.initialized) await this.init();
  }

  /**
   * Upsert an entity (create via POST).
   * The backend handles insert-or-update semantics.
   */
  async upsert(entity: Entity): Promise<void> {
    await this.ensureInit();
    const body = toNeo4jCreateBody(entity);
    await apiClient.post<ApiResponse<Neo4jEntityNode>>(`${ENTITY_API}/entities`, body);
  }

  /** Get an entity by ID */
  async get(id: string): Promise<Entity | null> {
    await this.ensureInit();
    try {
      const resp = await apiClient.get<ApiResponse<Neo4jEntityNode>>(
        `${ENTITY_API}/entities/${id}`
      );
      return fromNeo4jEntity(resp.data);
    } catch {
      return null;
    }
  }

  /** Get an entity by source system reference */
  async getBySource(source: EntitySource, sourceId: string): Promise<Entity | null> {
    await this.ensureInit();
    try {
      const params = new URLSearchParams({ source, sourceId, limit: '1' });
      const resp = await apiClient.get<ApiResponse<{ entities: Neo4jEntityNode[]; count: number }>>(
        `${ENTITY_API}/entities?${params}`
      );
      const node = resp.data.entities[0];
      return node ? fromNeo4jEntity(node) : null;
    } catch {
      return null;
    }
  }

  /** Search entities via server-side query */
  async search(query: string, types?: EntityType[], limit = 20): Promise<EntitySearchResult[]> {
    await this.ensureInit();
    const params = new URLSearchParams({ query, limit: String(limit) });
    if (types && types.length > 0) {
      params.set('types', types.join(','));
    }
    const resp = await apiClient.get<
      ApiResponse<{ entities: (Neo4jEntityNode & { score?: number })[]; count: number }>
    >(`${ENTITY_API}/entities?${params}`);

    return resp.data.entities.map(node => ({ ...fromNeo4jEntity(node), score: node.score ?? 1.0 }));
  }

  /** List entities by type with pagination */
  async list(entityType: EntityType, offset = 0, limit = 50): Promise<Entity[]> {
    await this.ensureInit();
    const params = new URLSearchParams({
      type: entityType,
      limit: String(limit),
      offset: String(offset),
    });
    const resp = await apiClient.get<ApiResponse<{ entities: Neo4jEntityNode[]; count: number }>>(
      `${ENTITY_API}/entities?${params}`
    );

    return resp.data.entities.map(fromNeo4jEntity);
  }

  /** Delete an entity (soft-delete on backend) */
  async delete(id: string): Promise<void> {
    await this.ensureInit();
    await apiClient.delete(`${ENTITY_API}/entities/${id}`);
  }

  /** Add a relationship between entities */
  async addRelation(relation: EntityRelation): Promise<void> {
    await this.ensureInit();
    const body = toNeo4jRelationBody(relation);
    await apiClient.post<ApiResponse<Neo4jRelationshipRecord>>(`${ENTITY_API}/relationships`, body);
  }

  /**
   * Get relationships for an entity.
   * @param entityId The entity to get relations for
   * @param direction "from" = outgoing, "to" = incoming, "both" = all (default)
   * @param relationType Optional filter by relation type
   */
  async getRelations(
    entityId: string,
    direction?: 'from' | 'to' | 'both',
    relationType?: RelationType
  ): Promise<EntityRelation[]> {
    await this.ensureInit();
    const resp = await apiClient.get<ApiResponse<{ relationships: Neo4jRelationshipRecord[] }>>(
      `${ENTITY_API}/entities/${entityId}/relationships`
    );

    let relations = resp.data.relationships.map(fromNeo4jRelation);

    // Filter by direction
    const dir = direction ?? 'both';
    if (dir === 'from') {
      relations = relations.filter(r => r.fromEntityId === entityId);
    } else if (dir === 'to') {
      relations = relations.filter(r => r.toEntityId === entityId);
    }

    // Filter by relation type
    if (relationType) {
      relations = relations.filter(r => r.relationType === relationType);
    }

    return relations;
  }

  /**
   * Tag an entity with optimistic concurrency control.
   * Uses If-Match with the entity's updatedAt to detect concurrent writes,
   * retrying up to MAX_TAG_RETRIES times on 409 Conflict.
   */
  async addTag(entityId: string, tag: string): Promise<void> {
    await this.ensureInit();
    for (let attempt = 0; attempt < MAX_TAG_RETRIES; attempt++) {
      const resp = await apiClient.get<ApiResponse<Neo4jEntityNode>>(
        `${ENTITY_API}/entities/${entityId}`
      );
      const node = resp.data;
      const props = parseProps(node.properties);
      const tags: string[] = Array.isArray(props.tags) ? props.tags : [];
      if (!tags.includes(tag)) {
        tags.push(tag);
      }
      props.tags = tags;

      try {
        await apiClient.put(
          `${ENTITY_API}/entities/${entityId}`,
          { properties: JSON.stringify(props) },
          { headers: { 'If-Match': node.updatedAt } }
        );
        return;
      } catch (err: unknown) {
        if (!isConflictError(err) || attempt === MAX_TAG_RETRIES - 1) throw err;
      }
    }
  }

  /**
   * Remove a tag with optimistic concurrency control.
   * Uses If-Match with the entity's updatedAt to detect concurrent writes,
   * retrying up to MAX_TAG_RETRIES times on 409 Conflict.
   */
  async removeTag(entityId: string, tag: string): Promise<void> {
    await this.ensureInit();
    for (let attempt = 0; attempt < MAX_TAG_RETRIES; attempt++) {
      const resp = await apiClient.get<ApiResponse<Neo4jEntityNode>>(
        `${ENTITY_API}/entities/${entityId}`
      );
      const node = resp.data;
      const props = parseProps(node.properties);
      const tags: string[] = Array.isArray(props.tags) ? props.tags : [];
      props.tags = tags.filter(t => t !== tag);

      try {
        await apiClient.put(
          `${ENTITY_API}/entities/${entityId}`,
          { properties: JSON.stringify(props) },
          { headers: { 'If-Match': node.updatedAt } }
        );
        return;
      } catch (err: unknown) {
        if (!isConflictError(err) || attempt === MAX_TAG_RETRIES - 1) throw err;
      }
    }
  }

  /** Find entities by tag, optionally filtered by type */
  async getByTag(tag: string, entityType?: EntityType): Promise<Entity[]> {
    await this.ensureInit();
    const params = new URLSearchParams({ tag, limit: '500' });
    if (entityType) {
      params.set('type', entityType);
    }
    const resp = await apiClient.get<ApiResponse<{ entities: Neo4jEntityNode[]; count: number }>>(
      `${ENTITY_API}/entities?${params}`
    );

    return resp.data.entities.map(fromNeo4jEntity);
  }
}
