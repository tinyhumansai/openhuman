/**
 * describeNode — a short, dynamic plain-language line for a workflow node card
 * ("GET https://…", "Every 5 minutes", "If status → true / false"), derived
 * from the node's live config so the card explains what it will do at a glance
 * without opening the config drawer. Falls back to a generic per-kind label
 * when the config isn't filled in yet.
 *
 * Pure + dependency-light (only {@link describeSchedule}) so it's trivially
 * testable and can be called on every render of `FlowNodeComponent`. Takes
 * `t` (and `locale`, forwarded to {@link describeSchedule} for weekday
 * formatting) as parameters rather than calling `useT()` itself — mirroring
 * `runStepSummary.ts` — so callers (React components that already hold a `t`
 * / `locale` from `useT()`) stay in control of localization.
 */
import { truncateText } from '../../utils/truncateText';
import { describeSchedule, type Translate } from './cron';
import type { NodeKind } from './types';

function str(config: Record<string, unknown>, key: string): string {
  const v = config[key];
  return typeof v === 'string' ? v.trim() : '';
}

function num(config: Record<string, unknown>, key: string): number | undefined {
  const v = config[key];
  return typeof v === 'number' && Number.isFinite(v) ? v : undefined;
}

/**
 * Fill `{name}` placeholders in one pass. Values are config text the user typed
 * (a prompt, a URL, an expression), so they must not be read as `replace`
 * patterns (`$$`, `$&`) or have their own `{…}` filled by a later placeholder.
 */
function fill(template: string, values: Record<string, string>): string {
  return template.replace(/\{(\w+)\}/g, (match, name: string) => values[name] ?? match);
}

/**
 * @param kind         the node kind (may be an unknown string for a future kind)
 * @param config       the node's free-form config object
 * @param outputPorts  effective output ports (used to hint branch routing)
 * @param t            translate function from `useT()`
 * @param locale       active locale from `useT()`, for weekday formatting
 */
export function describeNode(
  kind: NodeKind | string,
  config: Record<string, unknown>,
  outputPorts: string[] = [],
  t: Translate,
  locale: string
): string {
  switch (kind) {
    case 'trigger': {
      const tk = str(config, 'trigger_kind') || 'manual';
      if (tk === 'manual') return t('flows.nodeSummary.trigger.manual');
      if (tk === 'schedule') return describeSchedule(config.schedule, t, locale);
      if (tk === 'webhook') return t('flows.nodeSummary.trigger.webhook');
      if (tk === 'app_event') {
        const parts = [str(config, 'toolkit'), str(config, 'trigger_slug')].filter(Boolean);
        return parts.length
          ? fill(t('flows.nodeSummary.trigger.appEventOn'), { parts: parts.join(' · ') })
          : t('flows.nodeSummary.trigger.appEvent');
      }
      return fill(t('flows.nodeSummary.trigger.unknownKind'), { kind: tk });
    }
    case 'agent': {
      const prompt = str(config, 'prompt');
      const model = str(config, 'model');
      const modelLabel = model
        ? model.replace(/^hint:/, '')
        : t('flows.nodeSummary.agent.defaultModel');
      return prompt
        ? fill(t('flows.nodeSummary.agent.withPrompt'), {
            prompt: truncateText(prompt, 40),
            model: modelLabel,
          })
        : fill(t('flows.nodeSummary.agent.default'), { model: modelLabel });
    }
    case 'tool_call': {
      const slug = str(config, 'slug');
      if (str(config, 'provider') === 'openhuman' || slug.startsWith('oh:')) {
        const name = slug.replace(/^oh:/, '');
        return name
          ? fill(t('flows.nodeSummary.toolCall.runsNative'), { name })
          : t('flows.nodeSummary.toolCall.pickNative');
      }
      return slug
        ? fill(t('flows.nodeSummary.toolCall.runs'), { slug })
        : t('flows.nodeSummary.toolCall.pick');
    }
    case 'http_request': {
      const method = str(config, 'method') || 'GET';
      const url = str(config, 'url');
      return url
        ? fill(t('flows.nodeSummary.http.withUrl'), { method, url: truncateText(url, 40) })
        : fill(t('flows.nodeSummary.http.noUrl'), { method });
    }
    case 'code': {
      const lang = str(config, 'language') || 'javascript';
      return fill(t('flows.nodeSummary.code.runs'), { lang });
    }
    case 'condition': {
      const field = str(config, 'field');
      return field
        ? fill(t('flows.nodeSummary.condition.withField'), { field })
        : t('flows.nodeSummary.condition.default');
    }
    case 'switch': {
      const expr = str(config, 'expression') || str(config, 'field');
      const hasRoutes = outputPorts.length > 0;
      const count = String(outputPorts.length);
      if (expr) {
        return hasRoutes
          ? fill(t('flows.nodeSummary.switch.byExprWithRoutes'), { expr, count })
          : fill(t('flows.nodeSummary.switch.byExpr'), { expr });
      }
      return hasRoutes
        ? fill(t('flows.nodeSummary.switch.byValueWithRoutes'), { count })
        : t('flows.nodeSummary.switch.byValue');
    }
    case 'merge':
      return t('flows.nodeSummary.merge');
    case 'split_out': {
      const path = str(config, 'path');
      return path
        ? fill(t('flows.nodeSummary.splitOut.withPath'), { path })
        : t('flows.nodeSummary.splitOut.default');
    }
    case 'transform': {
      const set = config.set;
      const n = set && typeof set === 'object' && !Array.isArray(set) ? Object.keys(set).length : 0;
      if (n === 0) return t('flows.nodeSummary.transform.default');
      const key =
        n === 1
          ? 'flows.nodeSummary.transform.setFieldsSingular'
          : 'flows.nodeSummary.transform.setFieldsPlural';
      return fill(t(key), { n: String(n) });
    }
    case 'output_parser':
      return t('flows.nodeSummary.outputParser');
    case 'sub_workflow':
      return t('flows.nodeSummary.subWorkflow');
    case 'memory': {
      const operation = str(config, 'operation') || 'recall';
      const scope = str(config, 'scope');
      if (operation === 'flavour') {
        const flavour = str(config, 'flavour');
        return flavour
          ? fill(t('flows.nodeSummary.memory.flavourWith'), { flavour })
          : t('flows.nodeSummary.memory.flavour');
      }
      if (operation === 'people') return t('flows.nodeSummary.memory.people');
      if (operation === 'remember') return t('flows.nodeSummary.memory.remember');
      if (operation === 'forget') return t('flows.nodeSummary.memory.forget');
      if (operation === 'search') {
        return scope
          ? fill(t('flows.nodeSummary.memory.searchScoped'), { scope })
          : t('flows.nodeSummary.memory.search');
      }
      // recall
      return scope
        ? fill(t('flows.nodeSummary.memory.recallScoped'), { scope })
        : t('flows.nodeSummary.memory.recall');
    }
    case 'dedup': {
      const key = str(config, 'key');
      return key
        ? fill(t('flows.nodeSummary.dedup.withKey'), { key: truncateText(key, 40) })
        : t('flows.nodeSummary.dedup.default');
    }
    case 'loop': {
      // The cap is what an operator most needs to see at a glance, and the
      // engine applies its own default when the key is absent, so the summary
      // says so rather than going blank.
      const max = num(config, 'max_iterations');
      const condition = str(config, 'condition');
      if (condition) {
        return fill(t('flows.nodeSummary.loop.whileCondition'), {
          max: String(max ?? 25),
          condition: truncateText(condition, 30),
        });
      }
      return fill(t('flows.nodeSummary.loop.upTo'), { max: String(max ?? 25) });
    }
    default:
      return '';
  }
}
