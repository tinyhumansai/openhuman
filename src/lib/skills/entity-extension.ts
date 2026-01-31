/**
 * Entity Extension Registry
 *
 * Skills register new entity types and relation types here.
 * The base types in src/lib/ai/entities/types.ts remain but become extensible
 * through this registry.
 */

import type {
  SkillEntityDefinition,
  EntityTypeRegistration,
  RelationTypeRegistration,
  EntityBuilder,
} from "./types";
import createDebug from "debug";

const log = createDebug("app:skills:entities");

/** Base entity types from the core platform */
const BASE_ENTITY_TYPES = new Set([
  "contact",
  "chat",
  "message",
  "email",
  "wallet",
  "token",
  "transaction",
]);

/** Base relation types from the core platform */
const BASE_RELATION_TYPES = new Set([
  "member_of",
  "sent_by",
  "sent_to",
  "owns",
  "traded",
  "replied_to",
]);

export class EntityExtensionRegistry {
  /** Skill-registered entity types, keyed by type name */
  private entityTypes = new Map<string, EntityTypeRegistration>();
  /** Skill-registered relation types, keyed by type name */
  private relationTypes = new Map<string, RelationTypeRegistration>();
  /** Entity builders, keyed by skill ID */
  private builders = new Map<string, EntityBuilder[]>();

  /** Register entity types, relation types, and builders for a skill */
  registerSkillEntities(skillId: string, def: SkillEntityDefinition): void {
    if (def.entityTypes) {
      for (const reg of def.entityTypes) {
        this.entityTypes.set(reg.type, { ...reg, skillId });
        log("Registered entity type %s from skill %s", reg.type, skillId);
      }
    }

    if (def.relationTypes) {
      for (const reg of def.relationTypes) {
        this.relationTypes.set(reg.type, { ...reg, skillId });
        log("Registered relation type %s from skill %s", reg.type, skillId);
      }
    }

    if (def.builders && def.builders.length > 0) {
      this.builders.set(skillId, def.builders);
      log(
        "Registered %d entity builders from skill %s",
        def.builders.length,
        skillId,
      );
    }
  }

  /** Unregister all entity extensions for a skill */
  unregisterSkillEntities(skillId: string): void {
    // Remove entity types from this skill
    for (const [type, reg] of this.entityTypes) {
      if (reg.skillId === skillId) {
        this.entityTypes.delete(type);
      }
    }

    // Remove relation types from this skill
    for (const [type, reg] of this.relationTypes) {
      if (reg.skillId === skillId) {
        this.relationTypes.delete(type);
      }
    }

    // Remove builders
    this.builders.delete(skillId);

    log("Unregistered entity extensions for skill %s", skillId);
  }

  /** Check if an entity type is valid (base or skill-registered) */
  isValidEntityType(type: string): boolean {
    return BASE_ENTITY_TYPES.has(type) || this.entityTypes.has(type);
  }

  /** Check if a relation type is valid (base or skill-registered) */
  isValidRelationType(type: string): boolean {
    return BASE_RELATION_TYPES.has(type) || this.relationTypes.has(type);
  }

  /** Get all valid entity types (base + skill-registered) */
  getAllEntityTypes(): string[] {
    return [
      ...BASE_ENTITY_TYPES,
      ...this.entityTypes.keys(),
    ];
  }

  /** Get all valid relation types (base + skill-registered) */
  getAllRelationTypes(): string[] {
    return [
      ...BASE_RELATION_TYPES,
      ...this.relationTypes.keys(),
    ];
  }

  /** Get entity type registrations from skills only */
  getSkillEntityTypes(): EntityTypeRegistration[] {
    return Array.from(this.entityTypes.values());
  }

  /** Get relation type registrations from skills only */
  getSkillRelationTypes(): RelationTypeRegistration[] {
    return Array.from(this.relationTypes.values());
  }

  /** Get all entity builders, optionally filtered by source */
  getBuilders(source?: string): EntityBuilder[] {
    const all = Array.from(this.builders.values()).flat();
    if (source) {
      return all.filter((b) => b.source === source);
    }
    return all;
  }

  /** Clear all skill extensions */
  clear(): void {
    this.entityTypes.clear();
    this.relationTypes.clear();
    this.builders.clear();
  }
}
