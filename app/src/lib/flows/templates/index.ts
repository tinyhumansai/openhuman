/**
 * Curated `WorkflowGraph` templates (Phase 4c) — bundled as static JSON so the
 * new-workflow chooser and the Workflows empty-state gallery can offer a
 * one-click "start from a working example" path. Selecting a template calls
 * `flows_create` with that template's `graph`; the created flow then opens in
 * the editable canvas exactly like any other flow.
 *
 * Every bundled graph is structurally valid against the same rules
 * `openhuman.flows_validate` enforces (`tinyflows::validate`): unique node ids,
 * exactly one `trigger` node, and every edge referencing an existing node
 * (asserted in `templates.test.ts`). Node configs are realistic starting
 * points, not finished automations — the user is expected to fill in the
 * blanks (channel/tool slugs, URLs, prompts) in the canvas after creating.
 *
 * NO dynamic imports (repo rule): each template is a static `import` of its
 * `.json` file. Display strings (name / description / category) are NOT stored
 * here — they are i18n'd via `flows.templates.<id>.name` /
 * `flows.templates.<id>.description` / `flows.templates.category.<category>`
 * keys so the gallery never hardcodes English in JSX. This module only carries
 * the stable `id`, the `category` grouping, and the `graph` itself.
 */
import type { WorkflowGraph } from '../types';
import appEventRoute from './app-event-route.json';
import askAgent from './ask-agent.json';
import dailyDigest from './daily-digest.json';
import httpFetchParse from './http-fetch-parse.json';
import opusSonnetBrief from './opus-sonnet-brief.json';
import scheduledScrape from './scheduled-scrape.json';
import webhookTriage from './webhook-triage.json';

/**
 * Grouping shown as a section header / badge in the gallery. Each value has a
 * matching `flows.templates.category.<value>` i18n key.
 */
export type FlowTemplateCategory = 'scheduled' | 'triggered' | 'onDemand';

/**
 * One curated template. `id` is stable (used to derive its i18n display keys
 * and as a React key); `graph` is the ready-to-create `WorkflowGraph`. The
 * human-readable name/description are resolved at render time from i18n, so
 * they intentionally do not live on this object.
 */
export interface FlowTemplate {
  id: string;
  category: FlowTemplateCategory;
  graph: WorkflowGraph;
}

/**
 * The curated set, in gallery display order. Casting each imported JSON to
 * `WorkflowGraph` is safe because `templates.test.ts` asserts every graph is
 * structurally valid (single trigger, unique ids, edges reference real nodes).
 */
export const FLOW_TEMPLATES: FlowTemplate[] = [
  { id: 'daily-digest', category: 'scheduled', graph: dailyDigest as WorkflowGraph },
  { id: 'scheduled-scrape', category: 'scheduled', graph: scheduledScrape as WorkflowGraph },
  { id: 'webhook-triage', category: 'triggered', graph: webhookTriage as WorkflowGraph },
  { id: 'app-event-route', category: 'triggered', graph: appEventRoute as WorkflowGraph },
  { id: 'http-fetch-parse', category: 'onDemand', graph: httpFetchParse as WorkflowGraph },
  { id: 'ask-agent', category: 'onDemand', graph: askAgent as WorkflowGraph },
  { id: 'opus-sonnet-brief', category: 'onDemand', graph: opusSonnetBrief as WorkflowGraph },
];

/** i18n key for a template's display name (`flows.templates.<id>.name`). */
export function templateNameKey(id: string): string {
  return `flows.templates.${id}.name`;
}

/** i18n key for a template's description (`flows.templates.<id>.description`). */
export function templateDescriptionKey(id: string): string {
  return `flows.templates.${id}.description`;
}

/** i18n key for a category label (`flows.templates.category.<category>`). */
export function templateCategoryKey(category: FlowTemplateCategory): string {
  return `flows.templates.category.${category}`;
}
