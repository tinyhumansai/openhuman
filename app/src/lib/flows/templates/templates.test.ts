/**
 * Structural validity tests for the bundled Phase 4c templates. Mirrors the
 * exact checks `tinyflows::validate` (and therefore `openhuman.flows_validate`)
 * enforces — unique node ids, exactly one `trigger` node, every edge endpoint
 * referencing an existing node — so a template can never ship in a shape the
 * backend would reject at `flows_create` time. Also asserts each template
 * carries the i18n keys the gallery renders it with, and that ids are unique.
 */
import { describe, expect, it } from 'vitest';

import en from '../../i18n/en';
import type { WorkflowGraph } from '../types';
import {
  FLOW_TEMPLATES,
  templateCategoryKey,
  templateDescriptionKey,
  templateNameKey,
} from './index';

/** The structural subset of `tinyflows::validate` this frontend can assert. */
function structuralErrors(graph: WorkflowGraph): string[] {
  const errors: string[] = [];

  const ids = graph.nodes.map(n => n.id);
  const seen = new Set<string>();
  for (const id of ids) {
    if (seen.has(id)) errors.push(`duplicate node id: ${id}`);
    seen.add(id);
  }

  const triggers = graph.nodes.filter(n => n.kind === 'trigger');
  if (triggers.length === 0) errors.push('missing trigger node');
  if (triggers.length > 1) errors.push(`multiple trigger nodes: ${triggers.length}`);

  const idSet = new Set(ids);
  for (const edge of graph.edges) {
    if (!idSet.has(edge.from_node)) errors.push(`edge from unknown node: ${edge.from_node}`);
    if (!idSet.has(edge.to_node)) errors.push(`edge to unknown node: ${edge.to_node}`);
  }

  return errors;
}

describe('flow templates', () => {
  it('ships at least five curated templates', () => {
    expect(FLOW_TEMPLATES.length).toBeGreaterThanOrEqual(5);
  });

  it('has a unique id for every template', () => {
    const ids = FLOW_TEMPLATES.map(t => t.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it.each(FLOW_TEMPLATES.map(t => [t.id, t] as const))(
    'template %s is a structurally valid WorkflowGraph',
    (_id, template) => {
      const graph = template.graph;
      expect(graph.schema_version).toBe(1);
      expect(Array.isArray(graph.nodes)).toBe(true);
      expect(Array.isArray(graph.edges)).toBe(true);
      // Exactly one trigger, unique ids, edges reference existing nodes.
      expect(structuralErrors(graph)).toEqual([]);
      // Precisely one trigger (belt-and-suspenders over the helper above).
      expect(graph.nodes.filter(n => n.kind === 'trigger')).toHaveLength(1);
    }
  );

  it.each(FLOW_TEMPLATES.map(t => [t.id, t] as const))(
    'template %s has i18n name/description/category keys',
    (id, template) => {
      const dict = en as Record<string, string>;
      expect(dict[templateNameKey(id)]).toBeTruthy();
      expect(dict[templateDescriptionKey(id)]).toBeTruthy();
      expect(dict[templateCategoryKey(template.category)]).toBeTruthy();
    }
  );
});
