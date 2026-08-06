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

function truncate(value: string, max = 52): string {
  return value.length > max ? `${value.slice(0, max - 1)}…` : value;
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
          ? t('flows.nodeSummary.trigger.appEventOn').replace('{parts}', parts.join(' · '))
          : t('flows.nodeSummary.trigger.appEvent');
      }
      return t('flows.nodeSummary.trigger.unknownKind').replace('{kind}', tk);
    }
    case 'agent': {
      const prompt = str(config, 'prompt');
      const model = str(config, 'model');
      const modelLabel = model
        ? model.replace(/^hint:/, '')
        : t('flows.nodeSummary.agent.defaultModel');
      return prompt
        ? t('flows.nodeSummary.agent.withPrompt')
            .replace('{prompt}', truncate(prompt, 40))
            .replace('{model}', modelLabel)
        : t('flows.nodeSummary.agent.default').replace('{model}', modelLabel);
    }
    case 'tool_call': {
      const slug = str(config, 'slug');
      if (str(config, 'provider') === 'openhuman' || slug.startsWith('oh:')) {
        const name = slug.replace(/^oh:/, '');
        return name
          ? t('flows.nodeSummary.toolCall.runsNative').replace('{name}', name)
          : t('flows.nodeSummary.toolCall.pickNative');
      }
      return slug
        ? t('flows.nodeSummary.toolCall.runs').replace('{slug}', slug)
        : t('flows.nodeSummary.toolCall.pick');
    }
    case 'http_request': {
      const method = str(config, 'method') || 'GET';
      const url = str(config, 'url');
      return url
        ? t('flows.nodeSummary.http.withUrl')
            .replace('{method}', method)
            .replace('{url}', truncate(url, 40))
        : t('flows.nodeSummary.http.noUrl').replace('{method}', method);
    }
    case 'code': {
      const lang = str(config, 'language') || 'javascript';
      return t('flows.nodeSummary.code.runs').replace('{lang}', lang);
    }
    case 'condition': {
      const field = str(config, 'field');
      return field
        ? t('flows.nodeSummary.condition.withField').replace('{field}', field)
        : t('flows.nodeSummary.condition.default');
    }
    case 'switch': {
      const expr = str(config, 'expression') || str(config, 'field');
      const hasRoutes = outputPorts.length > 0;
      const count = String(outputPorts.length);
      if (expr) {
        return hasRoutes
          ? t('flows.nodeSummary.switch.byExprWithRoutes')
              .replace('{expr}', expr)
              .replace('{count}', count)
          : t('flows.nodeSummary.switch.byExpr').replace('{expr}', expr);
      }
      return hasRoutes
        ? t('flows.nodeSummary.switch.byValueWithRoutes').replace('{count}', count)
        : t('flows.nodeSummary.switch.byValue');
    }
    case 'merge':
      return t('flows.nodeSummary.merge');
    case 'split_out': {
      const path = str(config, 'path');
      return path
        ? t('flows.nodeSummary.splitOut.withPath').replace('{path}', path)
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
      return t(key).replace('{n}', String(n));
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
          ? t('flows.nodeSummary.memory.flavourWith').replace('{flavour}', flavour)
          : t('flows.nodeSummary.memory.flavour');
      }
      if (operation === 'people') return t('flows.nodeSummary.memory.people');
      if (operation === 'remember') return t('flows.nodeSummary.memory.remember');
      if (operation === 'forget') return t('flows.nodeSummary.memory.forget');
      if (operation === 'search') {
        return scope
          ? t('flows.nodeSummary.memory.searchScoped').replace('{scope}', scope)
          : t('flows.nodeSummary.memory.search');
      }
      // recall
      return scope
        ? t('flows.nodeSummary.memory.recallScoped').replace('{scope}', scope)
        : t('flows.nodeSummary.memory.recall');
    }
    case 'dedup': {
      const key = str(config, 'key');
      return key
        ? t('flows.nodeSummary.dedup.withKey').replace('{key}', truncate(key, 40))
        : t('flows.nodeSummary.dedup.default');
    }
    case 'loop': {
      // The cap is what an operator most needs to see at a glance, and the
      // engine applies its own default when the key is absent, so the summary
      // says so rather than going blank.
      const max = num(config, 'max_iterations');
      const condition = str(config, 'condition');
      if (condition) {
        return t('flows.nodeSummary.loop.whileCondition')
          .replace('{max}', String(max ?? 25))
          .replace('{condition}', truncate(condition, 30));
      }
      return t('flows.nodeSummary.loop.upTo').replace('{max}', String(max ?? 25));
    }
    default:
      return '';
  }
}
