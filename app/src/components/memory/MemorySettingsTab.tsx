/**
 * Memory → Settings: the recall policy behind the memory pack injected before
 * every turn (`memory_policy_get` / `memory_policy_set`) — whether recall runs,
 * the pack's token budget, how many learnings, brain documents, history and
 * team items it may hold, how often beliefs are built and how long a turn
 * waits for its pack. Also shows the layout root and agent id memory files
 * under, and says when the host pins them.
 *
 * Numbers save on blur/Enter, one field per call, and an out-of-range entry is
 * reverted rather than sent (the core rejects it anyway). The tab ends with the
 * erase-all-memory control (`MemoryEraseAllCard`).
 *
 * debug logging: DEBUG=openhuman:memory:settings
 */
import debug from 'debug';
import { useCallback, useEffect, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import {
  memoryErrorMessage,
  type MemoryPolicy,
  memoryPolicyGet,
  memoryPolicySet,
  type PolicyUpdate,
} from '../../services/api/memoryApi';
import { Card, NumberField, Switch } from '../ui';
import { CenteredLoadingState } from '../ui/LoadingState';
import MemoryEraseAllCard from './MemoryEraseAllCard';
import MemoryErrorAlert from './MemoryErrorAlert';
import { parseIntInRange } from './memoryFormat';

const log = debug('openhuman:memory:settings');

type NumericField =
  | 'budget_tokens'
  | 'learnings_limit'
  | 'brain_limit'
  | 'history_limit'
  | 'team_limit'
  | 'build_beliefs_every'
  | 'pre_turn_timeout_ms';

interface FieldSpec {
  field: NumericField;
  min: number;
  max: number;
  label: string;
  help: string;
  unit?: string;
}

const FIELDS: readonly NumericField[] = [
  'budget_tokens',
  'learnings_limit',
  'brain_limit',
  'history_limit',
  'team_limit',
  'build_beliefs_every',
  'pre_turn_timeout_ms',
];

function draftsFrom(policy: MemoryPolicy): Record<NumericField, string> {
  const out = {} as Record<NumericField, string>;
  for (const field of FIELDS) out[field] = String(policy.recall?.[field] ?? '');
  return out;
}

interface MemorySettingsTabProps {
  /** Called after all memory was erased, so the page re-reads memory state. */
  onMemoryErased?: () => void;
}

export default function MemorySettingsTab({ onMemoryErased }: MemorySettingsTabProps = {}) {
  const { t } = useT();
  const [policy, setPolicy] = useState<MemoryPolicy | null>(null);
  const [drafts, setDrafts] = useState<Record<NumericField, string> | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const specs: FieldSpec[] = [
    {
      field: 'budget_tokens',
      min: 100,
      max: 16000,
      label: t('memoryPage.settings.budget'),
      help: t('memoryPage.settings.budgetHelp'),
      unit: t('memoryPage.settings.tokens'),
    },
    {
      field: 'learnings_limit',
      min: 0,
      max: 50,
      label: t('memoryPage.settings.learningsLimit'),
      help: t('memoryPage.settings.limitHelp'),
    },
    {
      field: 'brain_limit',
      min: 0,
      max: 50,
      label: t('memoryPage.settings.brainLimit'),
      help: t('memoryPage.settings.limitHelp'),
    },
    {
      field: 'history_limit',
      min: 0,
      max: 50,
      label: t('memoryPage.settings.historyLimit'),
      help: t('memoryPage.settings.limitHelp'),
    },
    {
      field: 'team_limit',
      min: 0,
      max: 50,
      label: t('memoryPage.settings.teamLimit'),
      help: t('memoryPage.settings.limitHelp'),
    },
    {
      field: 'build_beliefs_every',
      min: 0,
      max: 1000,
      label: t('memoryPage.settings.buildBeliefsEvery'),
      help: t('memoryPage.settings.buildBeliefsHelp'),
      unit: t('memoryPage.settings.turns'),
    },
    {
      field: 'pre_turn_timeout_ms',
      min: 100,
      max: 30000,
      label: t('memoryPage.settings.preTurnTimeout'),
      help: t('memoryPage.settings.preTurnTimeoutHelp'),
      unit: t('memoryPage.settings.ms'),
    },
  ];

  const apply = useCallback((next: MemoryPolicy) => {
    setPolicy(next);
    setDrafts(draftsFrom(next));
  }, []);

  useEffect(() => {
    let cancelled = false;
    memoryPolicyGet()
      .then(next => {
        if (!cancelled) apply(next);
      })
      .catch(err => {
        if (cancelled) return;
        log('policy_get failed: %o', err);
        setError(memoryErrorMessage(err, t));
      });
    return () => {
      cancelled = true;
    };
  }, [apply, t]);

  const save = async (update: PolicyUpdate) => {
    setSaving(true);
    setError(null);
    try {
      log('policy_set: %o', update);
      apply(await memoryPolicySet(update));
    } catch (err) {
      log('policy_set failed: %o', err);
      setError(memoryErrorMessage(err, t));
      // Show the stored values again, not the rejected entry.
      setDrafts(prev => (policy ? draftsFrom(policy) : prev));
    } finally {
      setSaving(false);
    }
  };

  const commit = (spec: FieldSpec) => {
    if (!policy || !drafts) return;
    const stored = policy.recall?.[spec.field];
    const value = parseIntInRange(drafts[spec.field], spec.min, spec.max);
    if (value === null) {
      setDrafts({ ...drafts, [spec.field]: String(stored ?? '') });
      return;
    }
    if (value === stored) return;
    void save({ [spec.field]: value });
  };

  if (policy === null || drafts === null) {
    return error !== null ? (
      <MemoryErrorAlert message={error} data-testid="memory-settings-error" />
    ) : (
      <CenteredLoadingState label={t('memoryPage.loading')} />
    );
  }

  const recallOn = policy.recall?.enabled ?? false;

  return (
    <div className="space-y-4 animate-fade-up" data-testid="memory-settings-tab">
      {error !== null && <MemoryErrorAlert message={error} data-testid="memory-settings-error" />}

      <Card
        title={t('memoryPage.settings.recallTitle')}
        description={t('memoryPage.settings.recallDescription')}>
        <div className="flex items-center justify-between gap-4 px-4 py-3">
          <label htmlFor="memory-settings-recall" className="text-sm text-content">
            {t('memoryPage.settings.recallEnabled')}
          </label>
          <Switch
            id="memory-settings-recall"
            data-testid="memory-settings-recall"
            checked={recallOn}
            disabled={saving}
            aria-label={t('memoryPage.settings.recallEnabled')}
            onCheckedChange={next => void save({ recall_enabled: next })}
          />
        </div>
        {specs.map(spec => (
          <div key={spec.field} className="flex items-center justify-between gap-4 px-4 py-3">
            <div className="min-w-0">
              <p className="text-sm text-content">{spec.label}</p>
              <p className="text-xs text-content-muted">{spec.help}</p>
            </div>
            <NumberField
              id={`memory-settings-${spec.field}`}
              data-testid={`memory-settings-${spec.field}`}
              aria-label={spec.label}
              value={drafts[spec.field]}
              min={spec.min}
              max={spec.max}
              unit={spec.unit}
              disabled={saving || (!recallOn && spec.field !== 'build_beliefs_every')}
              onChange={value => setDrafts({ ...drafts, [spec.field]: value })}
              onCommit={() => commit(spec)}
            />
          </div>
        ))}
      </Card>

      <Card
        title={t('memoryPage.settings.identityTitle')}
        description={t('memoryPage.settings.identityDescription')}
        data-testid="memory-settings-identity">
        <dl className="divide-y divide-line-subtle text-sm">
          <div className="flex items-center justify-between gap-4 px-4 py-3">
            <dt className="text-content">{t('memoryPage.settings.root')}</dt>
            <dd className="truncate font-mono text-xs text-content-muted">{policy.root}</dd>
          </div>
          <div className="flex items-center justify-between gap-4 px-4 py-3">
            <dt className="text-content">{t('memoryPage.settings.agentId')}</dt>
            <dd className="truncate font-mono text-xs text-content-muted">{policy.agent_id}</dd>
          </div>
        </dl>
        {policy.host_bound && (
          <p
            className="px-4 pb-3 text-xs text-content-muted"
            data-testid="memory-settings-host-bound">
            {t('memoryPage.settings.hostBound')}
          </p>
        )}
      </Card>

      <MemoryEraseAllCard onErased={onMemoryErased} />
    </div>
  );
}
