import type { EntityManager } from './manager';
import type { Entity, EntitySearchResult, EntitySource, EntityType, RelationType } from './types';

/**
 * Fluent query builder for entity search and traversal.
 *
 * Usage:
 * ```ts
 * const results = await new EntityQuery(entityManager)
 *   .ofType("contact")
 *   .fromSource("telegram")
 *   .withTag("vip")
 *   .search("alice");
 *
 * const related = await new EntityQuery(entityManager)
 *   .relatedTo(entityId, "member_of")
 *   .execute();
 * ```
 */
export class EntityQuery {
  private manager: EntityManager;
  private typeFilter?: EntityType;
  private sourceFilter?: EntitySource;
  private tagFilter?: string;
  private searchQuery?: string;
  private relatedEntityId?: string;
  private relatedDirection?: 'from' | 'to' | 'both';
  private relatedRelationType?: RelationType;
  private limitValue = 50;
  private offsetValue = 0;

  constructor(manager: EntityManager) {
    this.manager = manager;
  }

  /** Filter by entity type */
  ofType(type: EntityType): this {
    this.typeFilter = type;
    return this;
  }

  /** Filter by source system */
  fromSource(source: EntitySource): this {
    this.sourceFilter = source;
    return this;
  }

  /** Filter by tag */
  withTag(tag: string): this {
    this.tagFilter = tag;
    return this;
  }

  /** Set full-text search query */
  matching(query: string): this {
    this.searchQuery = query;
    return this;
  }

  /** Find entities related to a given entity */
  relatedTo(
    entityId: string,
    relationType?: RelationType,
    direction?: 'from' | 'to' | 'both'
  ): this {
    this.relatedEntityId = entityId;
    this.relatedRelationType = relationType;
    this.relatedDirection = direction;
    return this;
  }

  /** Set result limit */
  limit(n: number): this {
    this.limitValue = n;
    return this;
  }

  /** Set result offset for pagination */
  offset(n: number): this {
    this.offsetValue = n;
    return this;
  }

  /**
   * Execute the query based on configured filters.
   *
   * Priority:
   * 1. If relatedTo is set, return related entities
   * 2. If searchQuery is set, use FTS search
   * 3. If tagFilter is set, use tag lookup
   * 4. Otherwise, list by type
   */
  async execute(): Promise<Entity[]> {
    // Related entity traversal
    if (this.relatedEntityId) {
      const relations = await this.manager.getRelations(
        this.relatedEntityId,
        this.relatedDirection,
        this.relatedRelationType
      );

      // Collect related entity IDs
      const relatedIds = new Set<string>();
      for (const rel of relations) {
        if (rel.fromEntityId !== this.relatedEntityId) {
          relatedIds.add(rel.fromEntityId);
        }
        if (rel.toEntityId !== this.relatedEntityId) {
          relatedIds.add(rel.toEntityId);
        }
      }

      // Fetch each related entity
      const entities: Entity[] = [];
      for (const id of relatedIds) {
        const entity = await this.manager.get(id);
        if (entity && this.matchesFilters(entity)) {
          entities.push(entity);
        }
      }

      return entities.slice(this.offsetValue, this.offsetValue + this.limitValue);
    }

    // FTS search
    if (this.searchQuery) {
      const types = this.typeFilter ? [this.typeFilter] : undefined;
      const results = await this.manager.search(this.searchQuery, types, this.limitValue);
      return results.filter(e => this.matchesFilters(e));
    }

    // Tag-based lookup
    if (this.tagFilter) {
      return this.manager.getByTag(this.tagFilter, this.typeFilter);
    }

    // List by type (type is required for list)
    if (this.typeFilter) {
      return this.manager.list(this.typeFilter, this.offsetValue, this.limitValue);
    }

    return [];
  }

  /**
   * Execute the query and return results with search scores.
   * Only works when a search query is set.
   */
  async search(query?: string): Promise<EntitySearchResult[]> {
    const q = query ?? this.searchQuery;
    if (!q) return [];

    const types = this.typeFilter ? [this.typeFilter] : undefined;
    return this.manager.search(q, types, this.limitValue);
  }

  /** Check if an entity matches the configured filters */
  private matchesFilters(entity: Entity): boolean {
    if (this.typeFilter && entity.type !== this.typeFilter) return false;
    if (this.sourceFilter && entity.source !== this.sourceFilter) return false;
    return true;
  }
}
