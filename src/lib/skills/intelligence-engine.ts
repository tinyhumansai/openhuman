/**
 * Intelligence Engine — Cross-Protocol Data Linking
 *
 * Evaluates rules from skills for autonomous entity linking and actions.
 *
 * Event flow:
 *   1. Something happens (entity created, message received, state changed)
 *   2. fireEvent() is called with event type + data
 *   3. Engine checks all registered rules for matching triggers
 *   4. Matching rules execute their actions
 *   5. Cooldown prevents duplicate firings
 */

import type {
  IntelligenceRule,
  IntelligenceActionContext,
} from "./types";
import type { EntityManager } from "../ai/entities/manager";
import createDebug from "debug";

const log = createDebug("app:skills:intelligence");

interface RegisteredRule {
  rule: IntelligenceRule;
  skillId: string;
  lastFiredAt: number;
}

export class IntelligenceEngine {
  private rules = new Map<string, RegisteredRule>();
  private entityManager: EntityManager | null = null;

  /** Set the entity manager for action execution */
  setEntityManager(entityManager: EntityManager): void {
    this.entityManager = entityManager;
  }

  /** Register intelligence rules for a skill */
  registerRules(skillId: string, rules: IntelligenceRule[]): void {
    for (const rule of rules) {
      const id = `${skillId}:${rule.id}`;
      this.rules.set(id, {
        rule,
        skillId,
        lastFiredAt: 0,
      });
      log("Registered intelligence rule %s from skill %s", rule.id, skillId);
    }
  }

  /** Unregister all rules for a skill */
  unregisterRules(skillId: string): void {
    for (const [id, entry] of this.rules) {
      if (entry.skillId === skillId) {
        this.rules.delete(id);
      }
    }
    log("Unregistered intelligence rules for skill %s", skillId);
  }

  /**
   * Fire an event and evaluate matching rules.
   * Rules that match the event trigger will have their actions executed.
   */
  async fireEvent(eventType: string, data: unknown): Promise<void> {
    const now = Date.now();
    const matchingRules: RegisteredRule[] = [];

    for (const entry of this.rules.values()) {
      const { rule, lastFiredAt } = entry;

      // Skip disabled rules
      if (rule.enabled === false) continue;

      // Check event type match
      if (rule.trigger.eventType !== eventType) continue;

      // Check filter conditions
      if (rule.trigger.filter) {
        const { entityType, source, predicate } = rule.trigger.filter;
        const eventData = data as Record<string, unknown>;

        if (entityType && eventData.entityType !== entityType) continue;
        if (source && eventData.source !== source) continue;
        if (predicate && !predicate(data)) continue;
      }

      // Check cooldown
      if (rule.cooldownMs && now - lastFiredAt < rule.cooldownMs) {
        log(
          "Rule %s skipped (cooldown: %dms remaining)",
          rule.id,
          rule.cooldownMs - (now - lastFiredAt),
        );
        continue;
      }

      matchingRules.push(entry);
    }

    // Execute matching rules
    for (const entry of matchingRules) {
      try {
        await this.executeAction(entry.rule, data);
        entry.lastFiredAt = now;
        log("Rule %s fired for event %s", entry.rule.id, eventType);
      } catch (error) {
        log(
          "Rule %s failed: %s",
          entry.rule.id,
          error instanceof Error ? error.message : String(error),
        );
      }
    }
  }

  /** Execute a rule's action */
  private async executeAction(
    rule: IntelligenceRule,
    data: unknown,
  ): Promise<void> {
    const ctx: IntelligenceActionContext = {
      entities: this.entityManager!,
      log: (message: string) => log("[rule:%s] %s", rule.id, message),
    };

    const { action } = rule;

    switch (action.type) {
      case "create_entity": {
        if (!this.entityManager) break;
        const entityData = data as Record<string, unknown>;
        const params = action.params ?? {};
        await this.entityManager.upsert({
          id: (params.id as string) ?? crypto.randomUUID(),
          type: (params.entityType as string) ?? (entityData.entityType as string) ?? "contact",
          source: (params.source as string) ?? (entityData.source as string) ?? "manual",
          sourceId: (params.sourceId as string) ?? null,
          title: (params.title as string) ?? null,
          summary: (params.summary as string) ?? null,
          metadata: params.metadata ? JSON.stringify(params.metadata) : null,
          createdAt: Date.now(),
          updatedAt: Date.now(),
        });
        break;
      }

      case "create_relation": {
        if (!this.entityManager) break;
        const params = action.params ?? {};
        await this.entityManager.addRelation({
          id: crypto.randomUUID(),
          fromEntityId: params.fromEntityId as string,
          toEntityId: params.toEntityId as string,
          relationType: (params.relationType as string) ?? "member_of",
          metadata: params.metadata ? JSON.stringify(params.metadata) : null,
          createdAt: Date.now(),
        });
        break;
      }

      case "tag_entity": {
        if (!this.entityManager) break;
        const params = action.params ?? {};
        const entityId = params.entityId as string;
        const tag = params.tag as string;
        if (entityId && tag) {
          await this.entityManager.addTag(entityId, tag);
        }
        break;
      }

      case "custom": {
        if (action.handler) {
          await action.handler(data, ctx);
        }
        break;
      }
    }
  }

  /** Get all registered rule IDs */
  getRuleIds(): string[] {
    return Array.from(this.rules.keys());
  }

  /** Get count of registered rules */
  get size(): number {
    return this.rules.size;
  }

  /** Clear all rules */
  clear(): void {
    this.rules.clear();
  }
}
