import { sha256 } from '@noble/hashes/sha2.js';
import { bytesToHex } from '@noble/hashes/utils.js';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { useT } from '../lib/i18n/I18nContext';
import {
  type CoreActionRequestLifecycleEnvelope,
  createCoreActionRequestClient,
  extractYoupetErrorCode,
  extractYoupetErrorField,
} from '../services/api/coreActionRequestClient';
import {
  createVerifiedUserScopedStorage,
  getActiveUserId,
  type VerifiedUserScopedStorage,
} from '../store/userScopedStorage';

type DecisionAction = 'approve' | 'reject';
type PendingDecisions = Record<string, DecisionAction>;

/** Logical key (physical path is user-scoped via verified storage). */
const IDEMPOTENCY_STORAGE_PREFIX = 'openhuman.youpet.action_request.idempotency.v4';
const DEFAULT_FILTER = 'pending';

export interface IntentStorageAdapter {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

export interface IntentScope {
  tenantId: string;
  /** OpenHuman active user id — never Core operator_user_id. */
  activeUserId: string;
}

const defaultVerifiedStorage: VerifiedUserScopedStorage = createVerifiedUserScopedStorage();

/**
 * Default adapter: repository user-scoping + verified durable writes.
 * Fail closed when no authenticated active user is set.
 */
const defaultUserScopedAdapter: IntentStorageAdapter = {
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

/** Overridable for deterministic fault-injection in tests. */
let storageAdapter: IntentStorageAdapter = defaultUserScopedAdapter;

export function setActionRequestIntentStorageAdapter(adapter: IntentStorageAdapter | null): void {
  storageAdapter = adapter ?? defaultUserScopedAdapter;
}

/** Logical storage key partitioned by tenant (user partition is physical). */
export function actionRequestIdempotencyStorageKey(tenantId: string): string {
  return `${IDEMPOTENCY_STORAGE_PREFIX}:${tenantId}`;
}

/**
 * Active OpenHuman user used for durable intent scoping.
 * Returns null when unauthenticated — callers must fail closed (no shared fallback).
 */
export function resolveActiveUserScope(): string | null {
  const id = getActiveUserId();
  if (!id || !id.trim()) return null;
  return id;
}

interface StoredIntent {
  key: string;
  action: DecisionAction;
  actionRequestId: string;
  /** SHA-256 fingerprint of the operator reason — never the raw reason text. */
  reasonFingerprint: string;
  expectedRowVersion: number;
}

type IntentStore = Record<string, StoredIntent>;

function isStoredIntent(value: unknown): value is StoredIntent {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const record = value as Record<string, unknown>;
  return (
    typeof record.key === 'string' &&
    record.key.trim().length > 0 &&
    (record.action === 'approve' || record.action === 'reject') &&
    typeof record.actionRequestId === 'string' &&
    typeof record.reasonFingerprint === 'string' &&
    record.reasonFingerprint.trim().length > 0 &&
    typeof record.expectedRowVersion === 'number' &&
    Number.isFinite(record.expectedRowVersion)
  );
}

function makeIdempotencyStorageId(actionRequestId: string, action: DecisionAction) {
  return `${action}:${actionRequestId}`;
}

function normalizeReason(reason: string) {
  return reason.trim();
}

/**
 * Collision-resistant fingerprint so raw operator intent is not persisted,
 * while distinct reasons never share an idempotency identity.
 */
export function fingerprintReason(reason: string): string {
  const normalized = normalizeReason(reason);
  const digest = sha256(new TextEncoder().encode(normalized));
  return `sha256:${bytesToHex(digest)}`;
}

function generateIdempotencyKey(actionRequestId: string, action: DecisionAction) {
  const random = globalThis.crypto?.randomUUID?.() ?? Math.random().toString(36).slice(2);
  return `youpet-action-request:${action}:${actionRequestId}:${random}`;
}

function readIdempotencyStore(tenantId: string): IntentStore {
  const storageKey = actionRequestIdempotencyStorageKey(tenantId);
  try {
    const raw = storageAdapter.getItem(storageKey);
    if (!raw) return {};
    const parsed = JSON.parse(raw);
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return {};
    const next: IntentStore = {};
    for (const [id, value] of Object.entries(parsed)) {
      if (typeof id === 'string' && isStoredIntent(value)) {
        next[id] = value;
      }
    }
    return next;
  } catch {
    throw new Error('idempotency_storage_read_failed');
  }
}

/** Persist intent store; returns false when the adapter cannot durable-write. */
function writeIdempotencyStore(tenantId: string, store: IntentStore): boolean {
  const storageKey = actionRequestIdempotencyStorageKey(tenantId);
  const serialized = JSON.stringify(store);
  try {
    storageAdapter.setItem(storageKey, serialized);
    // Verify round-trip so silent no-op adapters still fail closed.
    const roundTrip = storageAdapter.getItem(storageKey);
    if (roundTrip !== serialized) {
      return false;
    }
    return true;
  } catch {
    return false;
  }
}

export interface IdempotencyKeyResult {
  key: string;
  /** True only when the key is durable in scoped storage. */
  persisted: boolean;
}

function requireScope(scope?: IntentScope): IntentScope | null {
  if (scope?.tenantId && scope.activeUserId) return scope;
  const activeUserId = resolveActiveUserScope();
  if (!activeUserId) return null;
  if (scope?.tenantId) {
    return { tenantId: scope.tenantId, activeUserId };
  }
  return null;
}

/**
 * Return a stable idempotency key for the same complete operator intent.
 * Rotates when reason fingerprint or expected row version changes.
 * Callers must fail closed (do not call Core) when `persisted` is false.
 *
 * Requires an authenticated active-user scope; no shared `local-operator` fallback.
 */
export function getOrCreateIdempotencyKey(
  actionRequestId: string,
  action: DecisionAction,
  reason: string,
  expectedRowVersion: number,
  scope: IntentScope
): IdempotencyKeyResult {
  const resolved = requireScope(scope);
  if (!resolved) {
    return { key: '', persisted: false };
  }
  // Enforce that helper calls cannot write under a mismatched user identity
  // when using the default user-scoped adapter (active user drives the namespace).
  const liveUser = resolveActiveUserScope();
  if (
    liveUser &&
    liveUser !== resolved.activeUserId &&
    storageAdapter === defaultUserScopedAdapter
  ) {
    return { key: '', persisted: false };
  }

  let store: IntentStore;
  try {
    store = readIdempotencyStore(resolved.tenantId);
  } catch {
    return { key: '', persisted: false };
  }
  const id = makeIdempotencyStorageId(actionRequestId, action);
  const reasonFingerprint = fingerprintReason(reason);
  const existing = store[id];
  if (
    existing &&
    existing.reasonFingerprint === reasonFingerprint &&
    existing.expectedRowVersion === expectedRowVersion &&
    existing.action === action &&
    existing.actionRequestId === actionRequestId
  ) {
    return { key: existing.key, persisted: true };
  }
  const key = generateIdempotencyKey(actionRequestId, action);
  const nextStore: IntentStore = {
    ...store,
    [id]: { key, action, actionRequestId, reasonFingerprint, expectedRowVersion },
  };
  const persisted = writeIdempotencyStore(resolved.tenantId, nextStore);
  return { key, persisted };
}

export function clearIdempotencyKey(
  actionRequestId: string,
  action: DecisionAction,
  scope: IntentScope
): boolean {
  const resolved = requireScope(scope);
  if (!resolved) return false;
  let store: IntentStore;
  try {
    store = readIdempotencyStore(resolved.tenantId);
  } catch {
    return false;
  }
  const next = { ...store };
  delete next[makeIdempotencyStorageId(actionRequestId, action)];
  return writeIdempotencyStore(resolved.tenantId, next);
}

/** Clear both approve and reject intent keys for a request (terminal cleanup). */
export function clearAllDecisionIdempotencyKeys(
  actionRequestId: string,
  scope: IntentScope
): boolean {
  const resolved = requireScope(scope);
  if (!resolved) return false;
  let store: IntentStore;
  try {
    store = readIdempotencyStore(resolved.tenantId);
  } catch {
    return false;
  }
  const next = { ...store };
  delete next[makeIdempotencyStorageId(actionRequestId, 'approve')];
  delete next[makeIdempotencyStorageId(actionRequestId, 'reject')];
  return writeIdempotencyStore(resolved.tenantId, next);
}

function asRecord(value: unknown): Record<string, unknown> | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  return value as Record<string, unknown>;
}

function readString(value: unknown): string | null {
  return typeof value === 'string' && value.trim() ? value : null;
}

function readIdList(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return value
    .map(item => {
      if (typeof item === 'string' && item.trim()) return item;
      if (item && typeof item === 'object' && 'toString' in item) {
        const text = String(item);
        return text.trim() ? text : null;
      }
      return null;
    })
    .filter((item): item is string => Boolean(item));
}

function formatDate(value: string | null | undefined, noneLabel: string) {
  if (!value) return noneLabel;
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString();
}

function isPending(item: CoreActionRequestLifecycleEnvelope) {
  return item.approval_state === 'pending';
}

function isTerminalApproval(item: CoreActionRequestLifecycleEnvelope) {
  return (
    item.approval_state === 'approved' ||
    item.approval_state === 'rejected' ||
    item.approval_state === 'expired' ||
    item.approval_state === 'not_required'
  );
}

function matchesFilter(
  item: CoreActionRequestLifecycleEnvelope,
  filter: 'pending' | 'all'
): boolean {
  if (filter === 'all') return true;
  return isPending(item);
}

export default function ActionRequestInbox() {
  const { t } = useT();
  const client = useMemo(() => createCoreActionRequestClient(), []);
  const [items, setItems] = useState<CoreActionRequestLifecycleEnvelope[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [warning, setWarning] = useState<string | null>(null);
  const [reasonById, setReasonById] = useState<Record<string, string>>({});
  const [pending, setPending] = useState<PendingDecisions>({});
  const [filter, setFilter] = useState<'pending' | 'all'>(DEFAULT_FILTER);
  const inFlightRef = useRef<Set<string>>(new Set());
  /**
   * Data epoch: discard stale list *results* when a newer load or mutation
   * supersedes them. Intentionally separate from loading/refresh flag ownership
   * so invalidation never leaves the UI stuck refreshing.
   */
  const dataEpochRef = useRef(0);
  /** Owner token for the initial-load spinner. */
  const loadingOwnerRef = useRef(0);
  /** Owner token for the refresh button busy state. */
  const refreshingOwnerRef = useRef(0);
  const filterRef = useRef(filter);
  filterRef.current = filter;

  const selected = useMemo(
    () => items.find(item => item.id === selectedId) ?? null,
    [items, selectedId]
  );

  const mapError = useCallback(
    (err: unknown): string => {
      const code = extractYoupetErrorCode(err);
      const field = extractYoupetErrorField(err);
      if (field === 'tenant_id' || code === 'tenant_config_missing') {
        return t(
          'actionRequest.tenantConfigMissing',
          'YouPet tenant is not configured. Set YOUPET_TENANT_ID or youpet.tenant_id before listing ActionRequests.'
        );
      }
      if (code) {
        return t('actionRequest.errorWithCode', 'Action request failed ({code}).').replace(
          '{code}',
          code
        );
      }
      return t(
        'actionRequest.requestFailed',
        'Action request failed. Check Core configuration and try again.'
      );
    },
    [t]
  );

  const applyAuthoritativeItem = useCallback(
    (item: CoreActionRequestLifecycleEnvelope, activeFilter: 'pending' | 'all') => {
      if (!matchesFilter(item, activeFilter)) {
        setItems(current => current.filter(row => row.id !== item.id));
        setSelectedId(current => (current === item.id ? null : current));
        return;
      }
      setItems(current => {
        const exists = current.some(row => row.id === item.id);
        if (!exists) return [item, ...current];
        return current.map(row => (row.id === item.id ? item : row));
      });
    },
    []
  );

  const load = useCallback(
    async (mode: 'initial' | 'refresh' = 'initial') => {
      const dataEpoch = ++dataEpochRef.current;
      const loadOwner = mode === 'initial' ? ++loadingOwnerRef.current : loadingOwnerRef.current;
      const refreshOwner =
        mode === 'refresh' ? ++refreshingOwnerRef.current : refreshingOwnerRef.current;
      if (mode === 'initial') setLoading(true);
      else setRefreshing(true);
      setError(null);
      try {
        const listed = await client.list({
          approvalState: filter === 'pending' ? 'pending' : undefined,
          limit: 50,
        });
        // Stale data only — cleanup still runs in finally via owner tokens.
        if (dataEpoch !== dataEpochRef.current) return;
        setItems(listed);
        setSelectedId(current => {
          if (current && listed.some(item => item.id === current)) return current;
          return listed[0]?.id ?? null;
        });
      } catch (err) {
        if (dataEpoch !== dataEpochRef.current) return;
        setError(mapError(err));
      } finally {
        // Generation counters discard stale data, not stale cleanup.
        if (mode === 'initial' && loadOwner === loadingOwnerRef.current) {
          setLoading(false);
        }
        if (mode === 'refresh' && refreshOwner === refreshingOwnerRef.current) {
          setRefreshing(false);
        }
      }
    },
    [client, filter, mapError]
  );

  useEffect(() => {
    void load('initial');
  }, [load]);

  const refreshOne = useCallback(
    async (actionRequestId: string) => {
      const fresh = await client.get(actionRequestId);
      applyAuthoritativeItem(fresh, filterRef.current);
      return fresh;
    },
    [applyAuthoritativeItem, client]
  );

  const submitDecision = useCallback(
    async (item: CoreActionRequestLifecycleEnvelope, action: DecisionAction) => {
      const flightKey = `${action}:${item.id}`;
      if (inFlightRef.current.has(flightKey) || pending[item.id]) return;
      if (!isPending(item)) {
        setError(
          t('actionRequest.terminalReadOnly', 'This request is no longer pending and is read-only.')
        );
        return;
      }
      const reason = normalizeReason(reasonById[item.id] ?? '');
      if (!reason) {
        setError(t('actionRequest.reasonRequired', 'A non-empty operator reason is required.'));
        return;
      }

      const activeUserId = resolveActiveUserScope();
      if (!activeUserId) {
        setError(
          t(
            'actionRequest.storageUnavailable',
            'Local retry-key storage is unavailable. Decision blocked until storage works so retries stay idempotent.'
          )
        );
        return;
      }
      const scope: IntentScope = { tenantId: item.tenant_id, activeUserId };

      // Fail closed: only call Core when the intent key is durably persisted.
      let idempotencyKey: string;
      let persisted: boolean;
      try {
        ({ key: idempotencyKey, persisted } = getOrCreateIdempotencyKey(
          item.id,
          action,
          reason,
          item.row_version,
          scope
        ));
      } catch {
        setError(
          t(
            'actionRequest.storageUnavailable',
            'Local retry-key storage is unavailable. Decision blocked until storage works so retries stay idempotent.'
          )
        );
        return;
      }
      if (!persisted || !idempotencyKey) {
        setError(
          t(
            'actionRequest.storageUnavailable',
            'Local retry-key storage is unavailable. Decision blocked until storage works so retries stay idempotent.'
          )
        );
        return;
      }

      inFlightRef.current.add(flightKey);
      setPending(current => ({ ...current, [item.id]: action }));
      setError(null);
      setWarning(null);

      // Invalidate outstanding list *data* only. Do not steal loading/refresh
      // owner tokens — those finally blocks still clear their own busy flags.
      dataEpochRef.current += 1;

      try {
        const params = { reason, expectedRowVersion: item.row_version, idempotencyKey };
        const updated =
          action === 'approve'
            ? await client.approve(item.id, params)
            : await client.reject(item.id, params);

        // Immediate local apply for responsiveness, then authoritative Core get.
        applyAuthoritativeItem(updated, filterRef.current);
        setReasonById(current => {
          const next = { ...current };
          delete next[item.id];
          return next;
        });

        let authoritative = updated;
        try {
          authoritative = await client.get(item.id);
          applyAuthoritativeItem(authoritative, filterRef.current);
        } catch {
          // Mutation already succeeded; keep the response snapshot.
          setWarning(
            t(
              'actionRequest.refreshAfterMutationFailed',
              'Decision applied, but an authoritative Core refresh failed. Use Refresh to reload.'
            )
          );
        }

        if (isTerminalApproval(authoritative)) {
          const cleared = clearAllDecisionIdempotencyKeys(item.id, scope);
          if (!cleared) {
            setWarning(
              t(
                'actionRequest.storageWarning',
                'Local retry-key storage is unavailable; retry safety may be limited for this browser session.'
              )
            );
          }
        }
      } catch (err) {
        const code = extractYoupetErrorCode(err);
        if (code === 'concurrency_conflict' || code === 'idempotency_conflict') {
          try {
            const fresh = await refreshOne(item.id);
            setError(
              t(
                'actionRequest.conflictRefresh',
                'State changed ({code}). Reloaded from Core: {state} v{version}.'
              )
                .replace('{code}', code)
                .replace('{state}', fresh.approval_state)
                .replace('{version}', String(fresh.row_version))
            );
            if (isTerminalApproval(fresh)) {
              clearAllDecisionIdempotencyKeys(item.id, scope);
            }
          } catch {
            setError(
              t(
                'actionRequest.conflictRefreshFailed',
                'State conflict ({code}), and refresh from Core failed.'
              ).replace('{code}', code ?? 'conflict')
            );
          }
        } else {
          setError(mapError(err));
        }
      } finally {
        inFlightRef.current.delete(flightKey);
        setPending(current => {
          const next = { ...current };
          delete next[item.id];
          return next;
        });
      }
    },
    [applyAuthoritativeItem, client, mapError, pending, reasonById, refreshOne, t]
  );

  // Clear both action keys when loading a terminal request (stale intent hygiene).
  useEffect(() => {
    if (!selected || !isTerminalApproval(selected)) return;
    const activeUserId = resolveActiveUserScope();
    if (!activeUserId) return;
    clearAllDecisionIdempotencyKeys(selected.id, { tenantId: selected.tenant_id, activeUserId });
  }, [selected]);

  const doc = asRecord(selected?.action_request);
  const proposer = asRecord(doc?.proposer);
  const target = asRecord(doc?.target);
  const policy = asRecord(doc?.policy);
  const payload = asRecord(doc?.payload);
  const links = asRecord(doc?.links);
  const reasons = Array.isArray(policy?.reasons)
    ? policy.reasons.filter((r): r is string => typeof r === 'string')
    : [];
  const obligations = Array.isArray(policy?.obligations)
    ? policy.obligations.filter((r): r is string => typeof r === 'string')
    : [];
  const auditLogIds = readIdList(links?.audit_log_ids);
  const domainEventIds = readIdList(links?.domain_event_ids);
  const outboxDeliveryIds = readIdList(links?.outbox_delivery_ids);
  const workflowId = readString(links?.workflow_id);
  const workflowTraceId = readString(links?.workflow_trace_id);
  const agentRunId = readString(links?.agent_run_id);
  const proposalEventId = readString(links?.proposal_event_id);
  const linksIdempotencyKey = readString(links?.idempotency_key);
  const hasLinks =
    Boolean(links) &&
    (auditLogIds.length > 0 ||
      domainEventIds.length > 0 ||
      outboxDeliveryIds.length > 0 ||
      Boolean(workflowId) ||
      Boolean(workflowTraceId) ||
      Boolean(agentRunId) ||
      Boolean(proposalEventId) ||
      Boolean(linksIdempotencyKey));

  return (
    <div
      className="mx-auto flex w-full max-w-6xl flex-col gap-4 p-6"
      data-testid="action-request-inbox">
      <header className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <p className="text-xs uppercase tracking-wide text-zinc-500">
            {t('actionRequest.eyebrow')}
          </p>
          <h1 className="text-2xl font-semibold text-zinc-100">{t('actionRequest.title')}</h1>
          <p className="text-sm text-zinc-400">{t('actionRequest.subtitle')}</p>
        </div>
        <div className="flex items-center gap-2">
          <label className="text-sm text-zinc-400" htmlFor="ar-filter">
            {t('actionRequest.filterLabel')}
          </label>
          <select
            id="ar-filter"
            className="rounded border border-zinc-700 bg-zinc-900 px-2 py-1 text-sm text-zinc-100"
            value={filter}
            onChange={event => setFilter(event.target.value as 'pending' | 'all')}
            data-testid="action-request-filter">
            <option value="pending">{t('actionRequest.filter.pending')}</option>
            <option value="all">{t('actionRequest.filter.all')}</option>
          </select>
          <button
            type="button"
            className="rounded bg-zinc-800 px-3 py-1.5 text-sm text-zinc-100 hover:bg-zinc-700 disabled:opacity-50"
            onClick={() => void load('refresh')}
            disabled={loading || refreshing}
            data-testid="action-request-refresh">
            {refreshing ? t('actionRequest.refreshing') : t('actionRequest.refresh')}
          </button>
        </div>
      </header>

      {error ? (
        <div
          className="rounded border border-amber-700/60 bg-amber-950/40 px-3 py-2 text-sm text-amber-100"
          data-testid="action-request-error"
          role="alert">
          {error}
        </div>
      ) : null}

      {warning ? (
        <div
          className="rounded border border-zinc-600/60 bg-zinc-900/60 px-3 py-2 text-sm text-zinc-200"
          data-testid="action-request-warning"
          role="status">
          {warning}
        </div>
      ) : null}

      {loading ? (
        <p className="text-sm text-zinc-400" data-testid="action-request-loading">
          {t('actionRequest.loading')}
        </p>
      ) : null}

      {!loading && items.length === 0 ? (
        <p className="text-sm text-zinc-400" data-testid="action-request-empty">
          {t('actionRequest.empty')}
        </p>
      ) : null}

      <div className="grid gap-4 md:grid-cols-[minmax(0,1fr)_minmax(0,1.4fr)]">
        <ul className="flex flex-col gap-2" data-testid="action-request-list">
          {items.map(item => {
            const active = item.id === selectedId;
            return (
              <li key={item.id}>
                <button
                  type="button"
                  className={`w-full rounded border px-3 py-2 text-left text-sm ${
                    active
                      ? 'border-sky-600 bg-sky-950/40 text-sky-50'
                      : 'border-zinc-800 bg-zinc-900/60 text-zinc-200 hover:border-zinc-600'
                  }`}
                  onClick={() => setSelectedId(item.id)}
                  data-testid={`action-request-row-${item.id}`}>
                  <div className="font-medium">
                    {readString(asRecord(item.action_request)?.action_type) ?? item.id}
                  </div>
                  <div className="mt-1 text-xs text-zinc-400">
                    {item.approval_state} · {item.execution_state} · v{item.row_version}
                  </div>
                </button>
              </li>
            );
          })}
        </ul>

        <section
          className="rounded border border-zinc-800 bg-zinc-950/50 p-4"
          data-testid="action-request-detail">
          {!selected ? (
            <p className="text-sm text-zinc-400">{t('actionRequest.selectPrompt')}</p>
          ) : (
            <div className="flex flex-col gap-3 text-sm text-zinc-200">
              <div className="flex flex-wrap gap-2 text-xs text-zinc-400">
                <span data-testid="action-request-detail-id">{selected.id}</span>
                <span>
                  {t('actionRequest.rowVersion')}: {selected.row_version}
                </span>
                <span>
                  {t('actionRequest.approval')}: {selected.approval_state}
                </span>
                <span>
                  {t('actionRequest.execution')}: {selected.execution_state}
                </span>
              </div>

              <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1">
                <dt className="text-zinc-500">{t('actionRequest.actionType')}</dt>
                <dd>{readString(doc?.action_type) ?? t('actionRequest.none')}</dd>
                <dt className="text-zinc-500">{t('actionRequest.risk')}</dt>
                <dd>{readString(doc?.risk) ?? t('actionRequest.none')}</dd>
                <dt className="text-zinc-500">{t('actionRequest.proposer')}</dt>
                <dd>
                  {readString(proposer?.type) ?? t('actionRequest.none')}
                  {readString(proposer?.id) ? ` · ${readString(proposer?.id)}` : ''}
                </dd>
                <dt className="text-zinc-500">{t('actionRequest.target')}</dt>
                <dd>
                  {readString(target?.type) ?? t('actionRequest.none')}
                  {readString(target?.id) ? ` · ${readString(target?.id)}` : ''}
                </dd>
                <dt className="text-zinc-500">{t('actionRequest.policyOutcome')}</dt>
                <dd>{selected.policy_outcome}</dd>
                <dt className="text-zinc-500">{t('actionRequest.correlation')}</dt>
                <dd>{selected.correlation_id}</dd>
                <dt className="text-zinc-500">{t('actionRequest.updated')}</dt>
                <dd>{formatDate(selected.updated_at, t('actionRequest.none'))}</dd>
              </dl>

              {reasons.length > 0 ? (
                <div>
                  <h3 className="mb-1 text-xs uppercase tracking-wide text-zinc-500">
                    {t('actionRequest.reasons')}
                  </h3>
                  <ul className="list-disc pl-5 text-zinc-300">
                    {reasons.map(reason => (
                      <li key={reason}>{reason}</li>
                    ))}
                  </ul>
                </div>
              ) : null}

              {obligations.length > 0 ? (
                <div>
                  <h3 className="mb-1 text-xs uppercase tracking-wide text-zinc-500">
                    {t('actionRequest.obligations')}
                  </h3>
                  <ul className="list-disc pl-5 text-zinc-300">
                    {obligations.map(item => (
                      <li key={item}>{item}</li>
                    ))}
                  </ul>
                </div>
              ) : null}

              <div data-testid="action-request-links">
                <h3 className="mb-1 text-xs uppercase tracking-wide text-zinc-500">
                  {t('actionRequest.links', 'Links')}
                </h3>
                {hasLinks ? (
                  <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
                    {workflowId ? (
                      <>
                        <dt className="text-zinc-500">
                          {t('actionRequest.links.workflowId', 'Workflow ID')}
                        </dt>
                        <dd data-testid="action-request-link-workflow">{workflowId}</dd>
                      </>
                    ) : null}
                    {workflowTraceId ? (
                      <>
                        <dt className="text-zinc-500">
                          {t('actionRequest.links.workflowTraceId', 'Workflow trace ID')}
                        </dt>
                        <dd data-testid="action-request-link-workflow-trace">{workflowTraceId}</dd>
                      </>
                    ) : null}
                    {agentRunId ? (
                      <>
                        <dt className="text-zinc-500">
                          {t('actionRequest.links.agentRunId', 'Agent run ID')}
                        </dt>
                        <dd data-testid="action-request-link-agent-run">{agentRunId}</dd>
                      </>
                    ) : null}
                    {proposalEventId ? (
                      <>
                        <dt className="text-zinc-500">
                          {t('actionRequest.links.proposalEventId', 'Proposal event ID')}
                        </dt>
                        <dd data-testid="action-request-link-proposal">{proposalEventId}</dd>
                      </>
                    ) : null}
                    {linksIdempotencyKey ? (
                      <>
                        <dt className="text-zinc-500">
                          {t('actionRequest.links.idempotencyKey', 'Idempotency key')}
                        </dt>
                        <dd data-testid="action-request-link-idempotency">{linksIdempotencyKey}</dd>
                      </>
                    ) : null}
                    {auditLogIds.length > 0 ? (
                      <>
                        <dt className="text-zinc-500">
                          {t('actionRequest.links.auditLogIds', 'Audit log IDs')}
                        </dt>
                        <dd data-testid="action-request-link-audit">{auditLogIds.join(', ')}</dd>
                      </>
                    ) : null}
                    {domainEventIds.length > 0 ? (
                      <>
                        <dt className="text-zinc-500">
                          {t('actionRequest.links.domainEventIds', 'Domain event IDs')}
                        </dt>
                        <dd data-testid="action-request-link-domain">
                          {domainEventIds.join(', ')}
                        </dd>
                      </>
                    ) : null}
                    {outboxDeliveryIds.length > 0 ? (
                      <>
                        <dt className="text-zinc-500">
                          {t('actionRequest.links.outboxDeliveryIds', 'Outbox delivery IDs')}
                        </dt>
                        <dd data-testid="action-request-link-outbox">
                          {outboxDeliveryIds.join(', ')}
                        </dd>
                      </>
                    ) : null}
                  </dl>
                ) : (
                  <p className="text-xs text-zinc-500" data-testid="action-request-links-empty">
                    {t(
                      'actionRequest.linksEmpty',
                      'No correlation links recorded on this request.'
                    )}
                  </p>
                )}
              </div>

              {payload ? (
                <div>
                  <h3 className="mb-1 text-xs uppercase tracking-wide text-zinc-500">
                    {t('actionRequest.payload')}
                  </h3>
                  <pre
                    className="max-h-48 overflow-auto rounded bg-zinc-900 p-2 text-xs text-zinc-300"
                    data-testid="action-request-payload">
                    {JSON.stringify(payload, null, 2)}
                  </pre>
                </div>
              ) : null}

              {isPending(selected) ? (
                <div className="mt-2 flex flex-col gap-2 border-t border-zinc-800 pt-3">
                  <label className="text-xs text-zinc-400" htmlFor="ar-reason">
                    {t('actionRequest.reasonLabel')}
                  </label>
                  <textarea
                    id="ar-reason"
                    className="min-h-[72px] rounded border border-zinc-700 bg-zinc-900 px-2 py-1 text-sm text-zinc-100"
                    value={reasonById[selected.id] ?? ''}
                    onChange={event =>
                      setReasonById(current => ({ ...current, [selected.id]: event.target.value }))
                    }
                    data-testid="action-request-reason"
                  />
                  <div className="flex gap-2">
                    <button
                      type="button"
                      className="rounded bg-emerald-700 px-3 py-1.5 text-sm text-content-inverted hover:bg-emerald-600 disabled:opacity-50"
                      disabled={Boolean(pending[selected.id])}
                      onClick={() => void submitDecision(selected, 'approve')}
                      data-testid="action-request-approve">
                      {pending[selected.id] === 'approve'
                        ? t('actionRequest.approving')
                        : t('actionRequest.approve')}
                    </button>
                    <button
                      type="button"
                      className="rounded bg-rose-800 px-3 py-1.5 text-sm text-content-inverted hover:bg-rose-700 disabled:opacity-50"
                      disabled={Boolean(pending[selected.id])}
                      onClick={() => void submitDecision(selected, 'reject')}
                      data-testid="action-request-reject">
                      {pending[selected.id] === 'reject'
                        ? t('actionRequest.rejecting')
                        : t('actionRequest.reject')}
                    </button>
                  </div>
                </div>
              ) : (
                <p className="text-xs text-zinc-500" data-testid="action-request-terminal">
                  {t('actionRequest.terminalReadOnly')}
                </p>
              )}
            </div>
          )}
        </section>
      </div>
    </div>
  );
}
