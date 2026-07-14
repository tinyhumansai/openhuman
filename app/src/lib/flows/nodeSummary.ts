/**
 * describeNode — a short, dynamic plain-language line for a workflow node card
 * ("GET https://…", "Every 5 minutes", "If status → true / false"), derived
 * from the node's live config so the card explains what it will do at a glance
 * without opening the config drawer. Falls back to a generic per-kind label
 * when the config isn't filled in yet.
 *
 * Pure + dependency-light (only {@link describeSchedule}) so it's trivially
 * testable and can be called on every render of `FlowNodeComponent`.
 */
import { describeSchedule } from './cron';
import type { NodeKind } from './types';

function str(config: Record<string, unknown>, key: string): string {
  const v = config[key];
  return typeof v === 'string' ? v.trim() : '';
}

function truncate(value: string, max = 52): string {
  return value.length > max ? `${value.slice(0, max - 1)}…` : value;
}

/**
 * @param kind         the node kind (may be an unknown string for a future kind)
 * @param config       the node's free-form config object
 * @param outputPorts  effective output ports (used to hint branch routing)
 */
export function describeNode(
  kind: NodeKind | string,
  config: Record<string, unknown>,
  outputPorts: string[] = []
): string {
  switch (kind) {
    case 'trigger': {
      const tk = str(config, 'trigger_kind') || 'manual';
      if (tk === 'manual') return 'Runs on demand';
      if (tk === 'schedule') return describeSchedule(config.schedule);
      if (tk === 'webhook') return 'Runs on an incoming webhook';
      if (tk === 'app_event') {
        const parts = [str(config, 'toolkit'), str(config, 'trigger_slug')].filter(Boolean);
        return parts.length ? `On ${parts.join(' · ')}` : 'Runs on an app event';
      }
      return `Trigger: ${tk}`;
    }
    case 'agent': {
      const prompt = str(config, 'prompt');
      const model = str(config, 'model');
      const modelLabel = model ? model.replace(/^hint:/, '') : 'default model';
      return prompt ? `“${truncate(prompt, 40)}” · ${modelLabel}` : `Asks the ${modelLabel}`;
    }
    case 'tool_call': {
      const slug = str(config, 'slug');
      if (str(config, 'provider') === 'openhuman' || slug.startsWith('oh:')) {
        const name = slug.replace(/^oh:/, '');
        return name ? `Runs ${name}` : 'Runs an OpenHuman tool (pick one)';
      }
      return slug ? `Runs ${slug}` : 'Runs an app action (pick one)';
    }
    case 'http_request': {
      const method = str(config, 'method') || 'GET';
      const url = str(config, 'url');
      return url ? `${method} ${truncate(url, 40)}` : `${method} request (set a URL)`;
    }
    case 'code': {
      const lang = str(config, 'language') || 'javascript';
      return `Runs ${lang} code`;
    }
    case 'condition': {
      const field = str(config, 'field');
      return field ? `If ${field} → true / false` : 'Branches to true / false';
    }
    case 'switch': {
      const expr = str(config, 'expression') || str(config, 'field');
      const routes = outputPorts.length > 0 ? ` (${outputPorts.length} routes)` : '';
      return expr ? `Routes by ${expr}${routes}` : `Routes by a value${routes}`;
    }
    case 'merge':
      return 'Merges parallel branches';
    case 'split_out': {
      const path = str(config, 'path');
      return path ? `Splits each ${path}` : 'Splits a list into items';
    }
    case 'transform': {
      const set = config.set;
      const n = set && typeof set === 'object' && !Array.isArray(set) ? Object.keys(set).length : 0;
      return n > 0 ? `Sets ${n} field${n > 1 ? 's' : ''} on each item` : 'Reshapes each item';
    }
    case 'output_parser':
      return 'Parses the previous output';
    case 'sub_workflow':
      return 'Runs a nested workflow';
    default:
      return '';
  }
}
