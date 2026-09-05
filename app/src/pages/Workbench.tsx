import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { useT } from '../lib/i18n/I18nContext';
import {
  type CoreAlertSeverity,
  type CoreAlertStatus,
  type CoreWorkbenchAlert,
  type CoreWorkbenchAlertTrace,
  type CoreWorkbenchTraceActor,
  type CoreWorkbenchTraceEntry,
  type CoreWorkbenchTraceWarning,
  createCoreWorkbenchClient,
} from '../services/api/coreWorkbenchClient';
import {
  createVerifiedUserScopedStorage,
  getActiveUserId,
  type VerifiedUserScopedStorage,
} from '../store/userScopedStorage';

type StatusFilter = CoreAlertStatus | 'all';
type SeverityFilter = CoreAlertSeverity | 'all';
type AlertAction = 'ack' | 'resolve';
type PendingActions = Record<string, AlertAction>;

const STATUS_FILTERS: StatusFilter[] = ['open', 'acknowledged', 'resolved', 'dismissed', 'all'];
const SEVERITY_FILTERS: SeverityFilter[] = ['all', 'low', 'medium', 'high', 'critical'];
const IDEMPOTENCY_STORAGE_KEY = 'openhuman.youpet.workbench.idempotency.v1';
const STATUS_LABEL_KEYS: Record<StatusFilter, string> = {
  open: 'workbench.status.open',
  acknowledged: 'workbench.status.acknowledged',
  resolved: 'workbench.status.resolved',
  dismissed: 'workbench.status.dismissed',
  all: 'workbench.status.all',
};
const SEVERITY_LABEL_KEYS: Record<SeverityFilter, string> = {
  all: 'workbench.severity.all',
  low: 'workbench.severity.low',
  medium: 'workbench.severity.medium',
  high: 'workbench.severity.high',
  critical: 'workbench.severity.critical',
};
const MAX_METADATA_CHIPS = 6;
const MAX_METADATA_KEY_LENGTH = 48;
const MAX_METADATA_VALUE_LENGTH = 80;
const MAX_METADATA_COLLECTION_ITEMS = 8;
const MAX_METADATA_DEPTH = 2;
const MAX_METADATA_SERIALIZED_LENGTH = 512;

export const workbenchIdempotencyStorageKey = IDEMPOTENCY_STORAGE_KEY;

export interface WorkbenchIntentStorageAdapter {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

const defaultVerifiedStorage: VerifiedUserScopedStorage = createVerifiedUserScopedStorage();
const defaultUserScopedAdapter: WorkbenchIntentStorageAdapter = {
  getItem(key) {
    return defaultVerifiedStorage.getItem(key);
  },
  setItem(key, value) {
    defaultVerifiedStorage.setItem(key, value);
  },
  removeItem(key) {
    defaultVerifiedStorage.removeItem(key);
  },
};

let storageAdapter: WorkbenchIntentStorageAdapter = defaultUserScopedAdapter;

export function setWorkbenchIntentStorageAdapter(adapter: WorkbenchIntentStorageAdapter | null) {
  storageAdapter = adapter ?? defaultUserScopedAdapter;
}

export function resolveWorkbenchActiveUserScope(): string | null {
  const id = getActiveUserId();
  if (!id || !id.trim()) return null;
  return id;
}

function readIdempotencyStore(): Record<string, string> {
  try {
    const raw = storageAdapter.getItem(IDEMPOTENCY_STORAGE_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw);
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return {};
    return Object.fromEntries(
      Object.entries(parsed).filter((entry): entry is [string, string] => {
        const [key, value] = entry;
        return typeof key === 'string' && typeof value === 'string' && value.trim().length > 0;
      })
    );
  } catch {
    throw new Error('idempotency_storage_read_failed');
  }
}

function writeIdempotencyStore(store: Record<string, string>) {
  const serialized = JSON.stringify(store);
  try {
    storageAdapter.setItem(IDEMPOTENCY_STORAGE_KEY, serialized);
    return storageAdapter.getItem(IDEMPOTENCY_STORAGE_KEY) === serialized;
  } catch {
    return false;
  }
}

function makeIdempotencyStorageId(alertId: string, action: AlertAction) {
  return `${action}:${alertId}`;
}

function generateIdempotencyKey(alertId: string, action: AlertAction) {
  const random = globalThis.crypto?.randomUUID?.() ?? Math.random().toString(36).slice(2);
  return `youpet-workbench:${action}:${alertId}:${random}`;
}

function getOrCreateIdempotencyKey(alertId: string, action: AlertAction) {
  if (!resolveWorkbenchActiveUserScope()) {
    return { key: '', persisted: false };
  }
  let store: Record<string, string>;
  try {
    store = readIdempotencyStore();
  } catch {
    return { key: '', persisted: false };
  }
  const id = makeIdempotencyStorageId(alertId, action);
  if (store[id]) return { key: store[id], persisted: true };
  const key = generateIdempotencyKey(alertId, action);
  return { key, persisted: writeIdempotencyStore({ ...store, [id]: key }) };
}

function clearIdempotencyKey(alertId: string, action: AlertAction) {
  let store: Record<string, string>;
  try {
    store = readIdempotencyStore();
  } catch {
    return false;
  }
  delete store[makeIdempotencyStorageId(alertId, action)];
  return writeIdempotencyStore(store);
}

function formatDate(value: string | null | undefined, noneLabel: string) {
  if (!value) return noneLabel;
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString();
}

function formatOptional(value: string | null | undefined, noneLabel: string) {
  const trimmed = value?.trim();
  return trimmed ? trimmed : noneLabel;
}

function normalizeMetadataValue(value: unknown, depth = 0): unknown {
  if (Array.isArray(value)) {
    if (depth >= MAX_METADATA_DEPTH) return `[${value.length} items]`;
    const normalized = value
      .slice(0, MAX_METADATA_COLLECTION_ITEMS)
      .map(nested => normalizeMetadataValue(nested, depth + 1));
    return value.length > MAX_METADATA_COLLECTION_ITEMS
      ? [...normalized, `… ${value.length - MAX_METADATA_COLLECTION_ITEMS} more`]
      : normalized;
  }
  if (value && typeof value === 'object') {
    if (depth >= MAX_METADATA_DEPTH) return '[object]';
    const entries = Object.entries(value as Record<string, unknown>).sort(([left], [right]) =>
      left.localeCompare(right)
    );
    const normalized = entries
      .slice(0, MAX_METADATA_COLLECTION_ITEMS)
      .map(([key, nested]) => [key, normalizeMetadataValue(nested, depth + 1)]);
    if (entries.length > MAX_METADATA_COLLECTION_ITEMS) {
      normalized.push(['…', `${entries.length - MAX_METADATA_COLLECTION_ITEMS} more keys`]);
    }
    return Object.fromEntries(normalized);
  }
  return value;
}

function stringifyMetadataValue(value: unknown): string {
  if (value === null) return 'null';
  if (value === undefined) return '';
  if (typeof value === 'string') return truncateText(value, MAX_METADATA_SERIALIZED_LENGTH);
  if (typeof value === 'number' || typeof value === 'boolean') return String(value);
  try {
    return truncateText(
      JSON.stringify(normalizeMetadataValue(value)),
      MAX_METADATA_SERIALIZED_LENGTH
    );
  } catch {
    return truncateText(String(value), MAX_METADATA_SERIALIZED_LENGTH);
  }
}

function truncateText(value: string, limit = MAX_METADATA_VALUE_LENGTH): string {
  return value.length > limit ? `${value.slice(0, limit - 1)}…` : value;
}

function metadataChips(
  metadata: Record<string, unknown> | undefined,
  omitKeys: ReadonlySet<string> = new Set()
) {
  if (!metadata || typeof metadata !== 'object') return [];
  return Object.keys(metadata)
    .filter(key => !omitKeys.has(key))
    .sort((left, right) => left.localeCompare(right))
    .slice(0, MAX_METADATA_CHIPS)
    .map(
      key =>
        [
          key,
          truncateText(key, MAX_METADATA_KEY_LENGTH),
          truncateText(stringifyMetadataValue(metadata[key])),
        ] as const
    )
    .filter(([, , value]) => value.length > 0);
}

const ACTION_NAMED_METADATA_KEYS = new Set([
  'action_request_id',
  'action_type',
  'target_type',
  'target_id',
  'risk',
  'policy_outcome',
  'required_approver_class',
  'approval_state',
  'approver_class',
  'execution_state',
  'result_outcome_code',
  'error_code',
  'error_message',
]);

function metadataString(metadata: Record<string, unknown>, key: string): string | null {
  const value = metadata[key];
  if (typeof value === 'string' && value.trim()) return value;
  return null;
}

function labelFromLiteral(value: string): string {
  return value
    .split('_')
    .filter(Boolean)
    .map(part => part.charAt(0).toUpperCase() + part.slice(1))
    .join(' ');
}

function formatTraceActor(actor: CoreWorkbenchTraceActor | null | undefined, noneLabel: string) {
  if (!actor) return noneLabel;
  return actor.id ? `${labelFromLiteral(actor.type)} · ${actor.id}` : labelFromLiteral(actor.type);
}

function isActionRequestLifecycleKind(entry: CoreWorkbenchTraceEntry) {
  return (
    entry.kind === 'action_request_proposed' ||
    entry.kind === 'action_request_approved' ||
    entry.kind === 'action_request_rejected' ||
    entry.kind === 'action_request_execution'
  );
}

function traceLane(
  entry: CoreWorkbenchTraceEntry
): 'Step' | 'Action' | 'Event' | 'Delivery' | 'Audit' {
  if (isActionRequestLifecycleKind(entry)) {
    return 'Action';
  }
  if (
    entry.kind === 'health_plan_state' ||
    entry.kind === 'task_state' ||
    entry.kind === 'checkin_received'
  ) {
    return 'Step';
  }
  if (
    entry.kind === 'outbox_delivery' ||
    entry.kind === 'delivery_failed' ||
    entry.kind === 'delivery_succeeded' ||
    entry.kind === 'delivery_recovered' ||
    entry.kind === 'delivery_dead_lettered'
  ) {
    return 'Delivery';
  }
  if (entry.kind === 'audit_action' || entry.source === 'audit_logs') return 'Audit';
  if (entry.source === 'health_plans' || entry.source === 'task_instances') return 'Step';
  if (entry.source === 'outbox_deliveries') return 'Delivery';
  return 'Event';
}

function traceDeliveryStatus(entry: CoreWorkbenchTraceEntry): string | null {
  if (entry.kind === 'delivery_failed') return 'Failed · Retry scheduled';
  if (entry.kind === 'delivery_recovered') return 'Recovered';
  if (entry.kind === 'delivery_succeeded') return 'Succeeded';
  if (entry.kind === 'delivery_dead_lettered') return 'Dead lettered';
  if (entry.kind !== 'outbox_delivery') return null;
  const state = entry.metadata?.state;
  return typeof state === 'string' ? labelFromLiteral(state) : null;
}

function WorkbenchAlertContextPanel({
  alert,
  t,
}: {
  alert: CoreWorkbenchAlert;
  t: (key: string, fallback?: string) => string;
}) {
  const none = t('workbench.none');
  const context = alert.context;
  if (!context) {
    return (
      <section className="mt-4 rounded-lg border border-amber-200 bg-amber-50 p-3 text-sm text-amber-950 dark:border-amber-500/30 dark:bg-amber-500/10 dark:text-amber-100">
        {t('workbench.contextUnavailable')}
      </section>
    );
  }

  const latestCheckin = context.latest_checkin;

  return (
    <section
      aria-label={t('workbench.contextFor', 'Operational context for {alertId}').replace(
        '{alertId}',
        alert.id
      )}
      className="mt-4 grid gap-3 rounded-lg border border-line bg-surface-canvas p-3 text-sm   md:grid-cols-2">
      <div>
        <p className="text-xs font-semibold uppercase text-content-faint">
          {t('workbench.context.pet', 'Pet')}
        </p>
        <p className="font-medium text-content ">{context.pet.name}</p>
        <p className="text-content-muted ">
          {context.pet.species} · {context.pet.status}
          {context.pet.breed ? ` · ${context.pet.breed}` : ''}
        </p>
      </div>
      <div>
        <p className="text-xs font-semibold uppercase text-content-faint">
          {t('workbench.context.owner', 'Owner')}
        </p>
        <p className="font-medium text-content ">{context.owner.name}</p>
        <p className="text-content-muted ">
          {formatOptional(context.owner.phone, none)} · {context.owner.status}
        </p>
      </div>
      <div>
        <p className="text-xs font-semibold uppercase text-content-faint">
          {t('workbench.context.plan', 'Health plan')}
        </p>
        <p className="font-medium text-content ">{context.health_plan.title}</p>
        <p className="text-content-muted ">
          {context.health_plan.plan_type} · {context.health_plan.status}
        </p>
        <p className="break-all text-content-muted ">
          {t('workbench.context.flowId', 'Flow ID')}:{' '}
          {formatOptional(context.health_plan.openclaw_flow_id, none)}
        </p>
      </div>
      <div>
        <p className="text-xs font-semibold uppercase text-content-faint">
          {t('workbench.context.task', 'Task')}
        </p>
        <p className="font-medium text-content ">
          {context.task.status} · {t('workbench.context.missed', 'Missed')}:{' '}
          {context.task.missed_count}
        </p>
        <p className="text-content-muted ">
          {t('workbench.context.due', 'Due')}: {formatDate(context.task.due_at, none)}
        </p>
        <p className="break-all text-content-muted ">
          {t('workbench.context.flowId', 'Flow ID')}:{' '}
          {formatOptional(context.task.openclaw_flow_id, none)}
        </p>
      </div>
      <div className="md:col-span-2">
        <p className="text-xs font-semibold uppercase text-content-faint">
          {t('workbench.context.latestCheckin', 'Latest check-in')}
        </p>
        {latestCheckin ? (
          <>
            <p className="font-medium text-content ">{formatOptional(latestCheckin.text, none)}</p>
            <p className="text-content-muted ">
              {formatDate(latestCheckin.submitted_at, none)}
              {latestCheckin.status_tags.length > 0
                ? ` · ${latestCheckin.status_tags.join(', ')}`
                : ''}
            </p>
          </>
        ) : (
          <p className="text-content-muted ">{none}</p>
        )}
      </div>
    </section>
  );
}

function WorkbenchTraceDrawer({
  alert,
  trace,
  loading,
  refreshing,
  error,
  onClose,
  onRefresh,
  t,
}: {
  alert: CoreWorkbenchAlert;
  trace: CoreWorkbenchAlertTrace | null;
  loading: boolean;
  refreshing: boolean;
  error: string | null;
  onClose: () => void;
  onRefresh: () => void;
  t: (key: string, fallback?: string) => string;
}) {
  const none = t('workbench.none');
  const drawerRef = useRef<HTMLElement | null>(null);

  useEffect(() => {
    const previousFocus =
      document.activeElement instanceof HTMLElement ? document.activeElement : null;
    drawerRef.current?.focus();

    const focusableSelector = [
      'button:not([disabled])',
      '[href]',
      'input:not([disabled])',
      'select:not([disabled])',
      'textarea:not([disabled])',
      '[tabindex]:not([tabindex="-1"])',
    ].join(',');

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault();
        onClose();
        return;
      }

      if (event.key !== 'Tab' || !drawerRef.current) return;

      const focusable = Array.from(
        drawerRef.current.querySelectorAll<HTMLElement>(focusableSelector)
      ).filter(
        element =>
          !element.hasAttribute('disabled') && element.getAttribute('aria-hidden') !== 'true'
      );
      if (focusable.length === 0) {
        event.preventDefault();
        drawerRef.current.focus();
        return;
      }

      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      const activeElement =
        document.activeElement instanceof HTMLElement ? document.activeElement : null;
      if (activeElement === drawerRef.current || !drawerRef.current.contains(activeElement)) {
        event.preventDefault();
        (event.shiftKey ? last : first).focus();
      } else if (event.shiftKey && activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    };

    document.addEventListener('keydown', handleKeyDown);
    return () => {
      document.removeEventListener('keydown', handleKeyDown);
      previousFocus?.focus();
    };
  }, [onClose]);

  return (
    <div
      className="fixed inset-0 z-40 flex justify-end bg-surface-overlay/50"
      role="presentation"
      onMouseDown={event => {
        if (event.target === event.currentTarget) onClose();
      }}>
      <aside
        ref={drawerRef}
        role="dialog"
        aria-modal="true"
        tabIndex={-1}
        aria-label={t('workbench.trace.dialogLabel', 'Workflow trace for {alertId}').replace(
          '{alertId}',
          alert.id
        )}
        className="flex h-full w-full max-w-xl flex-col border-l border-line bg-surface shadow-xl  ">
        <header className="border-b border-line p-4 ">
          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0">
              <p className="text-xs font-semibold uppercase tracking-[0.16em] text-primary-600 dark:text-primary-400">
                {t('workbench.trace.eyebrow', 'Trace')}
              </p>
              <h2 className="mt-1 break-words text-lg font-semibold">
                {alert.summary || t('workbench.noSummary')}
              </h2>
              <p className="mt-1 break-all text-xs text-content-muted ">
                {alert.related_type} / {alert.related_id}
              </p>
            </div>
            <button
              type="button"
              onClick={onClose}
              className="min-h-9 rounded-lg border border-line-strong px-3 text-sm font-medium text-content-secondary hover:bg-surface-canvas   ">
              {t('workbench.trace.close', 'Close')}
            </button>
          </div>
          <div className="mt-3 flex justify-end">
            <button
              type="button"
              onClick={onRefresh}
              disabled={loading || refreshing}
              className="min-h-9 rounded-lg border border-line-strong px-3 text-sm font-medium text-content-secondary hover:bg-surface-canvas disabled:cursor-not-allowed disabled:opacity-60   ">
              {refreshing
                ? t('workbench.trace.refreshing', 'Refreshing trace')
                : t('workbench.trace.refresh', 'Refresh trace')}
            </button>
          </div>
        </header>

        <div className="flex-1 overflow-y-auto p-4">
          {loading ? (
            <p className="text-sm text-content-muted ">
              {t('workbench.trace.loading', 'Loading trace')}
            </p>
          ) : (
            <div className="space-y-4">
              {error ? (
                <div className="rounded-lg border border-coral-200 bg-coral-50 p-3 text-sm text-coral-700 dark:border-coral-500/30 dark:bg-coral-500/10 dark:text-coral-200">
                  {error}
                </div>
              ) : null}

              {trace ? (
                <>
                  <section
                    aria-label={t('workbench.trace.workflowSummary', 'Workflow summary')}
                    className="rounded-lg border border-primary-200 bg-primary-50 p-3 text-sm dark:border-primary-500/30 dark:bg-primary-500/10">
                    <p className="text-xs font-semibold uppercase tracking-[0.12em] text-primary-700 dark:text-primary-300">
                      {t('workbench.trace.workflowSummary', 'Workflow summary')}
                    </p>
                    {trace.workflow ? (
                      <div className="mt-2 grid gap-2 sm:grid-cols-2">
                        <div>
                          <p className="font-medium text-content ">
                            {alert.context?.health_plan.title ??
                              t('workbench.trace.healthPlan', 'Health plan')}
                          </p>
                          <p className="break-all text-content-muted ">
                            {trace.workflow.type} / {trace.workflow.id}
                          </p>
                        </div>
                        <div>
                          <p className="text-content-muted ">
                            {alert.context?.pet.name ??
                              t('workbench.trace.unknownPet', 'Unknown pet')}
                          </p>
                          <p className="break-all text-content-muted ">
                            {t('workbench.context.flowId', 'Flow ID')}:{' '}
                            {formatOptional(trace.workflow.openclaw_flow_id, none)}
                          </p>
                        </div>
                      </div>
                    ) : (
                      <p className="mt-2 text-content-muted ">
                        {t(
                          'workbench.trace.workflowUnavailable',
                          'Workflow identity is unavailable for this alert.'
                        )}
                      </p>
                    )}
                  </section>

                  {trace.partial && trace.warnings.length > 0 && (
                    <div className="space-y-2">
                      {trace.warnings.map(warning => (
                        <TraceWarningNotice
                          key={`${warning.code}:${warning.source ?? ''}`}
                          warning={warning}
                          t={t}
                        />
                      ))}
                    </div>
                  )}

                  {trace.entries.length === 0 ? (
                    <p className="rounded-lg border border-line p-4 text-sm text-content-muted  ">
                      {t('workbench.trace.empty', 'No trace entries available for this alert.')}
                    </p>
                  ) : (
                    <ol className="space-y-3">
                      {trace.entries.map(entry => (
                        <TraceEntryItem key={entry.id} entry={entry} none={none} t={t} />
                      ))}
                    </ol>
                  )}
                </>
              ) : null}
            </div>
          )}
        </div>
      </aside>
    </div>
  );
}

const TRACE_WARNING_TITLE_KEYS: Record<string, string> = {
  missing_related_action_request: 'workbench.trace.warning.missingRelatedActionRequest',
  action_request_links_truncated: 'workbench.trace.warning.actionRequestLinksTruncated',
  trace_reserved_budget_exceeded: 'workbench.trace.warning.traceReservedBudgetExceeded',
};

function TraceWarningNotice({
  warning,
  t,
}: {
  warning: CoreWorkbenchTraceWarning;
  t: (key: string, fallback?: string) => string;
}) {
  const titleKey = TRACE_WARNING_TITLE_KEYS[warning.code];
  const title = titleKey
    ? t(titleKey, labelFromLiteral(warning.code))
    : labelFromLiteral(warning.code);
  return (
    <div className="rounded-lg border border-amber-200 bg-amber-50 p-3 text-sm text-amber-950 dark:border-amber-500/30 dark:bg-amber-500/10 dark:text-amber-100">
      <p className="font-medium">{title}</p>
      <p>{warning.message}</p>
      {warning.source ? (
        <p className="text-xs opacity-80">{labelFromLiteral(warning.source)}</p>
      ) : null}
    </div>
  );
}

function formatActionTarget(metadata: Record<string, unknown>, none: string): string {
  const targetType = metadataString(metadata, 'target_type');
  const targetId = metadataString(metadata, 'target_id');
  if (targetType && targetId) return `${targetType} / ${targetId}`;
  return targetId ?? targetType ?? none;
}

function ActionRequestFields({
  entry,
  none,
  t,
}: {
  entry: CoreWorkbenchTraceEntry;
  none: string;
  t: (key: string, fallback?: string) => string;
}) {
  const metadata = entry.metadata ?? {};
  const fields: Array<[string, string]> = [];
  const actionRequestId = metadataString(metadata, 'action_request_id');
  const actionType = metadataString(metadata, 'action_type');
  const risk = metadataString(metadata, 'risk');
  const policyOutcome = metadataString(metadata, 'policy_outcome');
  const requiredApproverClass = metadataString(metadata, 'required_approver_class');
  const approvalState = metadataString(metadata, 'approval_state');
  const approverClass = metadataString(metadata, 'approver_class');
  const executionState = metadataString(metadata, 'execution_state');
  const resultOutcome = metadataString(metadata, 'result_outcome_code');
  const errorCode = metadataString(metadata, 'error_code');
  const errorMessage = metadataString(metadata, 'error_message');
  if (actionRequestId) {
    fields.push([t('workbench.trace.actionRequestId', 'Action request'), actionRequestId]);
  }
  if (actionType) {
    fields.push([t('workbench.trace.actionType', 'Action type'), actionType]);
  }
  if (metadataString(metadata, 'target_type') || metadataString(metadata, 'target_id')) {
    fields.push([t('workbench.trace.target', 'Target'), formatActionTarget(metadata, none)]);
  }
  if (risk) {
    fields.push([t('workbench.trace.risk', 'Risk'), risk]);
  }
  if (policyOutcome) {
    fields.push([t('workbench.trace.policyOutcome', 'Policy'), policyOutcome]);
  }
  if (requiredApproverClass) {
    fields.push([
      t('workbench.trace.requiredApproverClass', 'Required approver'),
      requiredApproverClass,
    ]);
  }
  if (approvalState) {
    fields.push([t('workbench.trace.approvalState', 'Approval'), approvalState]);
  }
  if (approverClass) {
    fields.push([t('workbench.trace.approverClass', 'Approver class'), approverClass]);
  }
  if (executionState) {
    fields.push([t('workbench.trace.executionState', 'Execution'), executionState]);
  }
  if (resultOutcome) {
    fields.push([t('workbench.trace.executionResult', 'Result'), resultOutcome]);
  }
  if (errorCode || errorMessage) {
    fields.push([
      t('workbench.trace.executionError', 'Error'),
      [errorCode, errorMessage].filter(Boolean).join(' · '),
    ]);
  }
  if (fields.length === 0) return null;
  return (
    <dl className="mt-3 grid gap-x-3 gap-y-1 text-xs text-content-muted  sm:grid-cols-2">
      {fields.map(([label, value]) => (
        <div key={label}>
          <dt className="uppercase text-content-faint">{label}</dt>
          <dd className="break-all">{value}</dd>
        </div>
      ))}
    </dl>
  );
}

function TraceEntryItem({
  entry,
  none,
  t,
}: {
  entry: CoreWorkbenchTraceEntry;
  none: string;
  t: (key: string, fallback?: string) => string;
}) {
  const isAction = isActionRequestLifecycleKind(entry);
  const chips = metadataChips(entry.metadata, isAction ? ACTION_NAMED_METADATA_KEYS : undefined);
  const lane = traceLane(entry);
  const laneLabel = isAction ? t('workbench.trace.lane.action', 'Action') : lane;
  const deliveryStatus = traceDeliveryStatus(entry);

  return (
    <li className="rounded-lg border border-line p-3 text-sm ">
      <div className="flex flex-wrap items-center gap-2">
        <span className="rounded-md bg-sage-50 px-2 py-1 text-xs font-semibold uppercase text-sage-700 dark:bg-sage-500/10 dark:text-sage-300">
          {laneLabel}
        </span>
        <span className="rounded-md bg-primary-50 px-2 py-1 text-xs font-semibold uppercase text-primary-700 dark:bg-primary-500/10 dark:text-primary-300">
          {labelFromLiteral(entry.kind)}
        </span>
        <span className="rounded-md bg-surface-muted px-2 py-1 text-xs font-semibold uppercase text-content-secondary  ">
          {labelFromLiteral(entry.source)}
        </span>
        {entry.severity ? (
          <span className="rounded-md bg-coral-50 px-2 py-1 text-xs font-semibold uppercase text-coral-700 dark:bg-coral-500/10 dark:text-coral-200">
            {labelFromLiteral(entry.severity)}
          </span>
        ) : null}
        {deliveryStatus ? (
          <span className="rounded-md bg-amber-50 px-2 py-1 text-xs font-semibold uppercase text-amber-800 dark:bg-amber-500/10 dark:text-amber-200">
            {deliveryStatus}
          </span>
        ) : null}
      </div>
      <p className="mt-2 font-medium text-content ">{entry.title}</p>
      <p className="text-xs text-content-muted ">{formatDate(entry.occurred_at, none)}</p>
      {entry.detail ? <p className="mt-2 text-content-secondary ">{entry.detail}</p> : null}
      <dl className="mt-3 grid gap-x-3 gap-y-1 text-xs text-content-muted  sm:grid-cols-2">
        <div>
          <dt className="uppercase text-content-faint">{t('workbench.trace.actor', 'Actor')}</dt>
          <dd className="break-all">{formatTraceActor(entry.actor, none)}</dd>
        </div>
        <div>
          <dt className="uppercase text-content-faint">
            {t('workbench.trace.related', 'Related')}
          </dt>
          <dd className="break-all">
            {entry.related_type && entry.related_id
              ? `${entry.related_type} / ${entry.related_id}`
              : none}
          </dd>
        </div>
      </dl>
      {isAction ? <ActionRequestFields entry={entry} none={none} t={t} /> : null}
      {chips.length > 0 ? (
        <div
          className="mt-3 flex flex-wrap gap-2"
          aria-label={t('workbench.trace.metadata', 'Metadata')}>
          {chips.map(([key, displayKey, value]) => (
            <span
              key={key}
              className="min-w-0 max-w-full break-all rounded-md bg-surface-muted px-2 py-1 text-xs text-content-secondary  ">
              <span className="font-semibold">{displayKey}</span>: {value}
            </span>
          ))}
        </div>
      ) : null}
    </li>
  );
}

const Workbench = () => {
  const { t } = useT();
  const client = useMemo(() => createCoreWorkbenchClient({ timeoutMs: 15_000 }), []);
  const [status, setStatus] = useState<StatusFilter>('open');
  const [severity, setSeverity] = useState<SeverityFilter>('all');
  const [alerts, setAlerts] = useState<CoreWorkbenchAlert[]>([]);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [pendingActions, setPendingActions] = useState<PendingActions>({});
  const [notes, setNotes] = useState<Record<string, string>>({});
  const [resolutions, setResolutions] = useState<Record<string, string>>({});
  const [traceAlertId, setTraceAlertId] = useState<string | null>(null);
  const [traceAlert, setTraceAlert] = useState<CoreWorkbenchAlert | null>(null);
  const [trace, setTrace] = useState<CoreWorkbenchAlertTrace | null>(null);
  const [traceLoading, setTraceLoading] = useState(false);
  const [traceRefreshing, setTraceRefreshing] = useState(false);
  const [traceError, setTraceError] = useState<string | null>(null);
  const alertsRequestSeq = useRef(0);
  const traceRequestSeq = useRef(0);
  const activeTraceAlertRef = useRef<string | null>(null);

  const loadAlerts = useCallback(
    async (mode: 'initial' | 'refresh' = 'refresh') => {
      const requestId = alertsRequestSeq.current + 1;
      alertsRequestSeq.current = requestId;
      if (mode === 'initial') {
        setLoading(true);
      } else {
        setRefreshing(true);
      }
      setError(null);
      try {
        const next = await client.listAlerts({
          status: status === 'all' ? null : status,
          severity: severity === 'all' ? undefined : severity,
        });
        if (alertsRequestSeq.current !== requestId) {
          return;
        }
        setAlerts(next);
      } catch {
        if (alertsRequestSeq.current !== requestId) {
          return;
        }
        setError(t('workbench.requestFailed'));
      } finally {
        if (alertsRequestSeq.current === requestId) {
          setLoading(false);
          setRefreshing(false);
        }
      }
    },
    [client, severity, status, t]
  );

  useEffect(() => {
    void loadAlerts('initial');
  }, [loadAlerts]);

  const runAction = async (alert: CoreWorkbenchAlert, action: AlertAction) => {
    if (!resolveWorkbenchActiveUserScope()) {
      setActionError(
        t(
          'actionRequest.storageUnavailable',
          'Local retry-key storage is unavailable. Decision blocked until storage works so retries stay idempotent.'
        )
      );
      return;
    }
    const { key: idempotencyKey, persisted } = getOrCreateIdempotencyKey(alert.id, action);
    if (!persisted || !idempotencyKey) {
      setActionError(
        t(
          'actionRequest.storageUnavailable',
          'Local retry-key storage is unavailable. Decision blocked until storage works so retries stay idempotent.'
        )
      );
      return;
    }
    setPendingActions(current => ({ ...current, [alert.id]: action }));
    setActionError(null);
    try {
      const updated =
        action === 'ack'
          ? await client.ackAlert(alert.id, {
              note: notes[alert.id]?.trim() || undefined,
              idempotencyKey,
            })
          : await client.resolveAlert(alert.id, {
              resolution: resolutions[alert.id]?.trim() || undefined,
              idempotencyKey,
            });
      clearIdempotencyKey(alert.id, action);
      setAlerts(current =>
        current.map(item =>
          item.id === updated.id ? { ...updated, context: updated.context ?? item.context } : item
        )
      );
      await loadAlerts('refresh');
    } catch {
      setActionError(t('workbench.requestFailed'));
    } finally {
      setPendingActions(current => {
        const { [alert.id]: _finished, ...rest } = current;
        return rest;
      });
    }
  };

  const loadTrace = useCallback(
    async (alertId: string, mode: 'initial' | 'refresh' = 'initial') => {
      const requestId = traceRequestSeq.current + 1;
      traceRequestSeq.current = requestId;
      activeTraceAlertRef.current = alertId;
      if (mode === 'initial') {
        setTraceLoading(true);
        setTrace(null);
      } else {
        setTraceRefreshing(true);
      }
      setTraceError(null);
      try {
        const nextTrace = await client.getAlertTrace(alertId);
        if (traceRequestSeq.current !== requestId || activeTraceAlertRef.current !== alertId) {
          return;
        }
        setTrace(nextTrace);
      } catch {
        if (traceRequestSeq.current !== requestId || activeTraceAlertRef.current !== alertId) {
          return;
        }
        setTraceError(t('workbench.trace.requestFailed', 'Trace request failed. Try again.'));
      } finally {
        if (traceRequestSeq.current === requestId && activeTraceAlertRef.current === alertId) {
          setTraceLoading(false);
          setTraceRefreshing(false);
        }
      }
    },
    [client, t]
  );

  const openTrace = (alert: CoreWorkbenchAlert) => {
    setTraceAlertId(alert.id);
    setTraceAlert(alert);
    setTrace(null);
    setTraceError(null);
    void loadTrace(alert.id, 'initial');
  };

  const closeTrace = useCallback(() => {
    traceRequestSeq.current += 1;
    activeTraceAlertRef.current = null;
    setTraceAlertId(null);
    setTraceAlert(null);
    setTrace(null);
    setTraceError(null);
    setTraceLoading(false);
    setTraceRefreshing(false);
  }, []);

  const refreshTrace = useCallback(() => {
    if (!traceAlertId) return;
    void loadTrace(traceAlertId, 'refresh');
  }, [loadTrace, traceAlertId]);

  const isAlertPending = (alertId: string) => Boolean(pendingActions[alertId]);
  const isActionPending = (alertId: string, action: AlertAction) =>
    pendingActions[alertId] === action;

  return (
    <div className="min-h-full bg-surface-canvas  text-content ">
      <main className="mx-auto flex w-full max-w-6xl flex-col gap-4 px-4 py-6">
        <header className="flex flex-col gap-3 border-b border-line pb-4  md:flex-row md:items-end md:justify-between">
          <div>
            <p className="text-xs font-semibold uppercase tracking-[0.18em] text-primary-600 dark:text-primary-400">
              {t('workbench.eyebrow')}
            </p>
            <h1 className="mt-1 text-2xl font-semibold">{t('workbench.title')}</h1>
          </div>
          <button
            type="button"
            onClick={() => void loadAlerts('refresh')}
            disabled={refreshing}
            className="inline-flex min-h-10 items-center justify-center rounded-lg border border-line-strong px-4 text-sm font-medium text-content-secondary transition-colors hover:bg-surface disabled:cursor-not-allowed disabled:opacity-60   ">
            {refreshing ? t('workbench.refreshing') : t('workbench.refresh')}
          </button>
        </header>

        <section className="grid gap-3 rounded-lg border border-line bg-surface p-4   md:grid-cols-2">
          <label className="flex flex-col gap-1 text-sm font-medium text-content-secondary ">
            {t('workbench.status')}
            <select
              value={status}
              onChange={event => setStatus(event.target.value as StatusFilter)}
              className="min-h-10 rounded-lg border border-line-strong bg-surface px-3 text-sm text-content   "
              aria-label={t('workbench.statusFilterLabel')}>
              {STATUS_FILTERS.map(option => (
                <option key={option} value={option}>
                  {t(STATUS_LABEL_KEYS[option])}
                </option>
              ))}
            </select>
          </label>

          <label className="flex flex-col gap-1 text-sm font-medium text-content-secondary ">
            {t('workbench.severity')}
            <select
              value={severity}
              onChange={event => setSeverity(event.target.value as SeverityFilter)}
              className="min-h-10 rounded-lg border border-line-strong bg-surface px-3 text-sm text-content   "
              aria-label={t('workbench.severityFilterLabel')}>
              {SEVERITY_FILTERS.map(option => (
                <option key={option} value={option}>
                  {t(SEVERITY_LABEL_KEYS[option])}
                </option>
              ))}
            </select>
          </label>
        </section>

        {error && (
          <div className="rounded-lg border border-coral-200 bg-coral-50 p-3 text-sm text-coral-700 dark:border-coral-500/30 dark:bg-coral-500/10 dark:text-coral-200">
            {error}
          </div>
        )}
        {actionError && (
          <div className="rounded-lg border border-coral-200 bg-coral-50 p-3 text-sm text-coral-700 dark:border-coral-500/30 dark:bg-coral-500/10 dark:text-coral-200">
            {actionError}
          </div>
        )}

        <section className="rounded-lg border border-line bg-surface  ">
          {loading ? (
            <div className="p-6 text-sm text-content-muted ">{t('workbench.loading')}</div>
          ) : alerts.length === 0 ? (
            <div className="p-6 text-sm text-content-muted ">{t('workbench.empty')}</div>
          ) : (
            <div className="divide-y divide-line ">
              {alerts.map(alert => (
                <article key={alert.id} className="grid gap-4 p-4 lg:grid-cols-[1fr_18rem]">
                  <div className="min-w-0">
                    <div className="mb-2 flex flex-wrap items-center gap-2">
                      <span className="rounded-md bg-surface-muted px-2 py-1 text-xs font-semibold uppercase text-content-secondary  ">
                        {alert.severity}
                      </span>
                      <span className="rounded-md bg-primary-50 px-2 py-1 text-xs font-semibold uppercase text-primary-700 dark:bg-primary-500/10 dark:text-primary-300">
                        {alert.status}
                      </span>
                      <span className="text-xs text-content-muted ">{alert.alert_type}</span>
                    </div>
                    <h2 className="text-base font-semibold">
                      {alert.summary || t('workbench.noSummary')}
                    </h2>
                    <dl className="mt-3 grid gap-x-4 gap-y-2 text-sm md:grid-cols-2">
                      <div>
                        <dt className="text-xs uppercase text-content-faint">
                          {t('workbench.related')}
                        </dt>
                        <dd className="break-all text-content-secondary ">
                          {alert.related_type} / {alert.related_id}
                        </dd>
                      </div>
                      <div>
                        <dt className="text-xs uppercase text-content-faint">
                          {t('workbench.created')}
                        </dt>
                        <dd>{formatDate(alert.created_at, t('workbench.none'))}</dd>
                      </div>
                      <div>
                        <dt className="text-xs uppercase text-content-faint">
                          {t('workbench.acknowledged')}
                        </dt>
                        <dd>{formatDate(alert.acknowledged_at, t('workbench.none'))}</dd>
                      </div>
                      <div>
                        <dt className="text-xs uppercase text-content-faint">
                          {t('workbench.resolved')}
                        </dt>
                        <dd>{formatDate(alert.resolved_at, t('workbench.none'))}</dd>
                      </div>
                    </dl>
                    <WorkbenchAlertContextPanel alert={alert} t={t} />
                  </div>

                  <div className="flex flex-col gap-3">
                    <label className="flex flex-col gap-1 text-sm font-medium text-content-secondary ">
                      {t('workbench.ackNote')}
                      <input
                        value={notes[alert.id] ?? ''}
                        onChange={event =>
                          setNotes(current => ({ ...current, [alert.id]: event.target.value }))
                        }
                        className="min-h-10 rounded-lg border border-line-strong bg-surface px-3 text-sm  "
                        aria-label={t('workbench.ackNoteFor').replace('{alertId}', alert.id)}
                      />
                    </label>
                    <button
                      type="button"
                      onClick={() => void runAction(alert, 'ack')}
                      disabled={isAlertPending(alert.id)}
                      className="min-h-10 rounded-lg bg-primary-600 px-3 text-sm font-medium text-content-inverted transition-colors hover:bg-primary-700 disabled:cursor-not-allowed disabled:opacity-60">
                      {isActionPending(alert.id, 'ack')
                        ? t('workbench.acknowledging')
                        : t('workbench.acknowledge')}
                    </button>

                    <label className="flex flex-col gap-1 text-sm font-medium text-content-secondary ">
                      {t('workbench.resolution')}
                      <input
                        value={resolutions[alert.id] ?? ''}
                        onChange={event =>
                          setResolutions(current => ({
                            ...current,
                            [alert.id]: event.target.value,
                          }))
                        }
                        className="min-h-10 rounded-lg border border-line-strong bg-surface px-3 text-sm  "
                        aria-label={t('workbench.resolutionFor').replace('{alertId}', alert.id)}
                      />
                    </label>
                    <button
                      type="button"
                      onClick={() => void runAction(alert, 'resolve')}
                      disabled={isAlertPending(alert.id)}
                      className="min-h-10 rounded-lg border border-line-strong px-3 text-sm font-medium text-content transition-colors hover:bg-surface-canvas disabled:cursor-not-allowed disabled:opacity-60   ">
                      {isActionPending(alert.id, 'resolve')
                        ? t('workbench.resolving')
                        : t('workbench.resolve')}
                    </button>
                    <button
                      type="button"
                      onClick={() => openTrace(alert)}
                      className="min-h-10 rounded-lg border border-line-strong px-3 text-sm font-medium text-content transition-colors hover:bg-surface-canvas   ">
                      {t('workbench.trace.button', 'Trace')}
                    </button>
                  </div>
                </article>
              ))}
            </div>
          )}
        </section>
        {traceAlert ? (
          <WorkbenchTraceDrawer
            alert={traceAlert}
            trace={trace}
            loading={traceLoading}
            refreshing={traceRefreshing}
            error={traceError}
            onClose={closeTrace}
            onRefresh={refreshTrace}
            t={t}
          />
        ) : null}
      </main>
    </div>
  );
};

export default Workbench;
