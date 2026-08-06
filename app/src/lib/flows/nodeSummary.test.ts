/**
 * Unit tests for describeNode — the dynamic per-node card summary. Asserts the
 * config-driven text for representative kinds plus the generic fallbacks when
 * config isn't filled in.
 *
 * `describeNode` takes a `t` stub (rather than calling `useT()`) — see the
 * module doc — mirroring `runStepSummary.test.ts`. The stub mirrors the real
 * `flows.nodeSummary.*` / `flows.cron.*` strings in `lib/i18n/en.ts`.
 */
import { describe, expect, it } from 'vitest';

import type { Translate } from './cron';
import { describeNode } from './nodeSummary';

const STRINGS: Record<string, string> = {
  'flows.nodeSummary.trigger.manual': 'Runs on demand',
  'flows.nodeSummary.trigger.webhook': 'Runs on an incoming webhook',
  'flows.nodeSummary.trigger.appEventOn': 'On {parts}',
  'flows.nodeSummary.trigger.appEvent': 'Runs on an app event',
  'flows.nodeSummary.trigger.unknownKind': 'Trigger: {kind}',
  'flows.nodeSummary.agent.defaultModel': 'default model',
  'flows.nodeSummary.agent.withPrompt': '“{prompt}” · {model}',
  'flows.nodeSummary.agent.default': 'Asks the {model}',
  'flows.nodeSummary.toolCall.runsNative': 'Runs {name}',
  'flows.nodeSummary.toolCall.pickNative': 'Runs an OpenHuman tool (pick one)',
  'flows.nodeSummary.toolCall.runs': 'Runs {slug}',
  'flows.nodeSummary.toolCall.pick': 'Runs an app action (pick one)',
  'flows.nodeSummary.http.withUrl': '{method} {url}',
  'flows.nodeSummary.http.noUrl': '{method} request (set a URL)',
  'flows.nodeSummary.code.runs': 'Runs {lang} code',
  'flows.nodeSummary.condition.withField': 'If {field} → true / false',
  'flows.nodeSummary.condition.default': 'Branches to true / false',
  'flows.nodeSummary.switch.byExpr': 'Routes by {expr}',
  'flows.nodeSummary.switch.byExprWithRoutes': 'Routes by {expr} ({count} routes)',
  'flows.nodeSummary.switch.byValue': 'Routes by a value',
  'flows.nodeSummary.switch.byValueWithRoutes': 'Routes by a value ({count} routes)',
  'flows.nodeSummary.merge': 'Merges parallel branches',
  'flows.nodeSummary.splitOut.withPath': 'Splits each {path}',
  'flows.nodeSummary.splitOut.default': 'Splits a list into items',
  'flows.nodeSummary.transform.default': 'Reshapes each item',
  'flows.nodeSummary.transform.setFieldsSingular': 'Sets {n} field on each item',
  'flows.nodeSummary.transform.setFieldsPlural': 'Sets {n} fields on each item',
  'flows.nodeSummary.outputParser': 'Parses the previous output',
  'flows.nodeSummary.subWorkflow': 'Runs a nested workflow',
  'flows.nodeSummary.memory.flavourWith': 'Reads the "{flavour}" flavour',
  'flows.nodeSummary.memory.flavour': 'Reads a memory flavour',
  'flows.nodeSummary.memory.people': 'Looks up people memory',
  'flows.nodeSummary.memory.remember': 'Remembers a value in this workflow',
  'flows.nodeSummary.memory.forget': 'Forgets a value from this workflow',
  'flows.nodeSummary.memory.searchScoped': 'Searches memory ({scope})',
  'flows.nodeSummary.memory.search': 'Searches memory',
  'flows.nodeSummary.memory.recallScoped': 'Recalls memory ({scope})',
  'flows.nodeSummary.memory.recall': 'Recalls memory',
  'flows.nodeSummary.dedup.withKey': 'Skips items already seen by {key}',
  'flows.nodeSummary.dedup.default': 'Skips items already processed',
  'flows.cron.customSchedule': 'Custom schedule ({expr})',
  'flows.cron.noScheduleSet': 'No schedule set',
  'flows.cron.weekdays': 'weekdays',
  'flows.cron.weekends': 'weekends',
  'flows.cron.everyMinute': 'Every minute',
  'flows.cron.everyMinuteOnDays': 'Every minute on {days}',
  'flows.cron.everyNMinutes': 'Every {n} minutes',
  'flows.cron.everyNMinutesOnDays': 'Every {n} minutes on {days}',
  'flows.cron.everyHour': 'Every hour',
  'flows.cron.everyHourOnDays': 'Every hour on {days}',
  'flows.cron.everyNHours': 'Every {n} hours',
  'flows.cron.everyNHoursOnDays': 'Every {n} hours on {days}',
  'flows.cron.everyDayAtTime': 'Every day at {time}',
  'flows.cron.atTimeOnDays': 'At {time} on {days}',
  'flows.cron.invalidInterval': 'Invalid interval',
  'flows.cron.dailyEvery24h': 'Daily (every 24h)',
  'flows.cron.everyNDays': 'Every {n} days',
  'flows.cron.everyNHoursShort': 'Every {n}h',
  'flows.cron.everyNMinutesShort': 'Every {n}m',
  'flows.cron.everySecond': 'Every second',
  'flows.cron.everyNSeconds': 'Every {n}s',
  'flows.cron.onceAtRaw': 'Once at {at}',
  'flows.cron.onceAt': 'Once at {at}',
};
const t: Translate = key => STRINGS[key] ?? key;
const locale = 'en';

describe('describeNode', () => {
  it('describes a schedule trigger via its cron', () => {
    expect(
      describeNode('trigger', { trigger_kind: 'schedule', schedule: '*/5 * * * 3' }, [], t, locale)
    ).toBe('Every 5 minutes on Wed');
    expect(describeNode('trigger', { trigger_kind: 'manual' }, [], t, locale)).toBe(
      'Runs on demand'
    );
  });

  it('describes a schedule trigger stored as a tagged `{kind:"every"}` schedule', () => {
    // Regression: the engine writes `config.schedule` as a tagged object
    // (`{kind:"every", every_ms}`), not a bare cron string — the summary must
    // not fall through to "No schedule set" for that real, configured shape.
    const summary = describeNode(
      'trigger',
      { trigger_kind: 'schedule', schedule: { kind: 'every', every_ms: 86_400_000 } },
      [],
      t,
      locale
    );
    expect(summary).not.toBe('No schedule set');
    expect(summary).toContain('24h');
  });

  it('still shows "No schedule set" for a genuinely unconfigured schedule trigger', () => {
    expect(describeNode('trigger', { trigger_kind: 'schedule' }, [], t, locale)).toBe(
      'No schedule set'
    );
  });

  it('describes an http_request from method + url', () => {
    expect(
      describeNode('http_request', { method: 'POST', url: 'https://api.x.com/v1' }, [], t, locale)
    ).toBe('POST https://api.x.com/v1');
    expect(describeNode('http_request', {}, [], t, locale)).toBe('GET request (set a URL)');
  });

  it('describes an agent by model hint', () => {
    expect(describeNode('agent', { model: 'hint:coding' }, [], t, locale)).toBe('Asks the coding');
    expect(
      describeNode('agent', { prompt: 'Summarize the thread', model: '' }, [], t, locale)
    ).toContain('Summarize the thread');
  });

  it('describes branch nodes and reflects output routes', () => {
    expect(describeNode('condition', { field: 'status' }, [], t, locale)).toBe(
      'If status → true / false'
    );
    expect(
      describeNode('switch', { expression: 'item.type' }, ['a', 'b', 'default'], t, locale)
    ).toBe('Routes by item.type (3 routes)');
  });

  it('falls back for tool_call / transform with empty config', () => {
    expect(describeNode('tool_call', {}, [], t, locale)).toBe('Runs an app action (pick one)');
    expect(describeNode('transform', { set: { a: '=1', b: '=2' } }, [], t, locale)).toBe(
      'Sets 2 fields on each item'
    );
  });

  it('returns empty string for an unknown kind', () => {
    expect(describeNode('time_travel', {}, [], t, locale)).toBe('');
  });

  it('gives memory search its own summary, distinct from recall', () => {
    expect(describeNode('memory', { operation: 'search', scope: 'user' }, [], t, locale)).toBe(
      'Searches memory (user)'
    );
    expect(describeNode('memory', { operation: 'recall', scope: 'user' }, [], t, locale)).toBe(
      'Recalls memory (user)'
    );
    expect(describeNode('memory', { operation: 'search' }, [], t, locale)).toBe('Searches memory');
  });

  it('describes dedup by its key expression, and falls back generically when unset', () => {
    expect(describeNode('dedup', { key: '=item.id' }, [], t, locale)).toBe(
      'Skips items already seen by =item.id'
    );
    expect(describeNode('dedup', {}, [], t, locale)).toBe('Skips items already processed');
  });
});
