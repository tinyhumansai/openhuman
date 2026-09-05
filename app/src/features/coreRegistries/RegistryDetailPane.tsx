import { useId, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import type {
  AgentRegistryAgent,
  ConnectorNormalizationContract,
  ConnectorRegistryBinding,
  ConnectorRegistryType,
  ToolRegistryToolDefinition,
  ToolRegistryToolEnablement,
} from '../../services/api/coreRegistriesClient';
import ReadOnlyJson from './ReadOnlyJson';
import type {
  RegistryDetailRef,
  RegistryDetailState,
  RegistryInspectionState,
  RegistryTab,
} from './types';

interface RegistryDetailPaneProps {
  activeTab: RegistryTab;
  detailState: RegistryDetailState;
  state: RegistryInspectionState;
  onOpenDetail: (detail: RegistryDetailRef) => void | Promise<void>;
}

type TranslateFn = (key: string, fallback?: string) => string;

function formatLiteral(value: string): string {
  return value
    .split(/[_-]/)
    .filter(Boolean)
    .map(part => part.charAt(0).toUpperCase() + part.slice(1))
    .join(' ');
}

function formatDate(t: TranslateFn, value: string | null | undefined): string {
  if (!value) {
    return t('registries.detail.notAvailable');
  }
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}

function shortFingerprint(value: string): string {
  return value.slice(0, 12);
}

function canResolveToolDefinition(
  state: RegistryInspectionState,
  toolKey: string,
  version: number
) {
  const collection = state.tabs.tools.collections.toolDefinitions;
  const match = collection.items.find(item => item.toolKey === toolKey && item.version === version);
  return { match, observation: collection.observation.kind };
}

function canResolveToolEnablement(
  state: RegistryInspectionState,
  toolKey: string,
  version: number
) {
  const collection = state.tabs.tools.collections.toolEnablements;
  const match = collection.items.find(item => item.toolKey === toolKey && item.version === version);
  return { match, observation: collection.observation.kind };
}

function canResolveConnectorType(
  state: RegistryInspectionState,
  connectorKey: string,
  version: number
) {
  const collection = state.tabs.connectors.collections.connectorTypes;
  const match = collection.items.find(
    item => item.connectorKey === connectorKey && item.version === version
  );
  return { match, observation: collection.observation.kind };
}

async function copyText(value: string) {
  if (typeof navigator === 'undefined' || !navigator.clipboard?.writeText) {
    return;
  }

  try {
    await navigator.clipboard.writeText(value);
  } catch {
    // Clipboard can be unavailable inside Tauri webviews; ignore.
  }
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="space-y-3 rounded-3xl border border-stone-200 bg-white p-4 dark:border-neutral-800 dark:bg-neutral-900">
      <h3 className="text-sm font-semibold text-stone-900 dark:text-neutral-100">{title}</h3>
      {children}
    </section>
  );
}

function FieldList({ entries }: { entries: Array<[label: string, value: React.ReactNode]> }) {
  return (
    <dl className="grid grid-cols-[auto,1fr] gap-x-3 gap-y-2 text-sm">
      {entries.map(([label, value]) => (
        <div key={label} className="contents">
          <dt className="text-stone-500 dark:text-neutral-400">{label}</dt>
          <dd className="min-w-0 text-stone-800 dark:text-neutral-100">{value}</dd>
        </div>
      ))}
    </dl>
  );
}

function FingerprintRow({ label, value }: { label: string; value: string }) {
  const { t } = useT();

  return (
    <div className="flex flex-wrap items-center gap-3 rounded-2xl border border-stone-200 bg-stone-50 px-3 py-2 dark:border-neutral-800 dark:bg-neutral-900">
      <span className="font-mono text-xs text-stone-700 dark:text-neutral-200">
        {label} · {shortFingerprint(value)}
      </span>
      <button
        type="button"
        onClick={() => {
          void copyText(value);
        }}
        aria-label={t('registries.detail.copyFingerprint')}
        className="inline-flex items-center rounded-xl border border-stone-200 px-2.5 py-1 text-xs font-medium text-stone-700 transition hover:bg-white dark:border-neutral-700 dark:text-neutral-200 dark:hover:bg-neutral-800">
        {t('registries.detail.copyFingerprint')}
      </button>
    </div>
  );
}

function summarizeSchema(t: TranslateFn, value: Record<string, unknown>): string {
  const schemaType = typeof value.type === 'string' ? value.type : null;
  const propertyCount =
    value.properties && typeof value.properties === 'object'
      ? Object.keys(value.properties as Record<string, unknown>).length
      : 0;
  const requiredCount = Array.isArray(value.required) ? value.required.length : 0;

  if (Object.keys(value).length === 0) {
    return t('registries.detail.emptySchemaObject');
  }

  const summary = [
    schemaType ? t('registries.detail.schemaType').replace('{type}', schemaType) : null,
    propertyCount > 0
      ? t('registries.detail.schemaProperties').replace('{count}', String(propertyCount))
      : null,
    requiredCount > 0
      ? t('registries.detail.schemaRequired').replace('{count}', String(requiredCount))
      : null,
  ]
    .filter(Boolean)
    .join(' · ');

  return summary || summarizeRecordShape(t, value);
}

function summarizeRecordShape(t: TranslateFn, value: Record<string, unknown>): string {
  const keys = Object.keys(value);
  if (keys.length === 0) {
    return t('registries.detail.noFields');
  }

  const sample = keys.slice(0, 3).join(', ');
  return keys.length > 3
    ? t('registries.detail.fieldsSampleMore')
        .replace('{count}', String(keys.length))
        .replace('{sample}', sample)
    : t('registries.detail.fieldsSample')
        .replace('{count}', String(keys.length))
        .replace('{sample}', sample);
}

function summarizeScalarRecord(t: TranslateFn, value: Record<string, unknown>): string {
  const scalarEntries = Object.entries(value).filter(
    (entry): entry is [string, string | number | boolean] =>
      typeof entry[1] === 'string' || typeof entry[1] === 'number' || typeof entry[1] === 'boolean'
  );
  if (scalarEntries.length === 0) {
    return summarizeRecordShape(t, value);
  }
  return scalarEntries
    .slice(0, 3)
    .map(([key, value]) => `${key}: ${String(value)}`)
    .join(' · ');
}

function summarizeNormalizationContracts(
  t: TranslateFn,
  contracts: ConnectorNormalizationContract[]
): string {
  if (contracts.length === 0) {
    return t('registries.detail.noNormalizationContracts');
  }

  return contracts
    .map(
      contract =>
        `${contract.evidenceFamily} -> ${contract.kernelEventType}@${contract.kernelEventSchemaVersion}`
    )
    .join(', ');
}

function CollapsibleJson({ label, value }: { label: string; value: unknown }) {
  const [expanded, setExpanded] = useState(false);
  const panelId = useId();

  return (
    <div className="rounded-2xl border border-stone-200 bg-stone-50 px-3 py-2 dark:border-neutral-800 dark:bg-neutral-950">
      <button
        type="button"
        aria-expanded={expanded}
        aria-controls={panelId}
        onClick={() => {
          setExpanded(current => !current);
        }}
        className="text-sm font-medium text-stone-800 dark:text-neutral-100">
        {label}
      </button>
      {expanded ? (
        <div id={panelId} className="mt-3">
          <ReadOnlyJson value={value} />
        </div>
      ) : null}
    </div>
  );
}

function ReferenceButton({
  label,
  detail,
  onOpenDetail,
}: {
  label: string;
  detail: RegistryDetailRef;
  onOpenDetail: (detail: RegistryDetailRef) => void | Promise<void>;
}) {
  return (
    <button
      type="button"
      onClick={() => {
        void onOpenDetail(detail);
      }}
      className="inline-flex items-center rounded-xl border border-primary-200 bg-primary-50 px-3 py-1.5 text-sm font-medium text-primary-700 transition hover:bg-primary-100">
      {label}
    </button>
  );
}

function UnresolvedReference({ label }: { label: string }) {
  const { t } = useT();

  return (
    <span className="inline-flex items-center rounded-xl border border-amber-200 bg-amber-50 px-3 py-1.5 text-sm text-amber-800">
      {t('registries.detail.unresolved').replace('{label}', label)}
    </span>
  );
}

function DeferredReference({ label }: { label: string }) {
  const { t } = useT();

  return (
    <span className="inline-flex items-center rounded-xl border border-stone-200 bg-stone-50 px-3 py-1.5 text-sm text-stone-600 dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-300">
      {t('registries.detail.deferred').replace('{label}', label)}
    </span>
  );
}

function AgentDetail({
  record,
  state,
  onOpenDetail,
}: {
  record: AgentRegistryAgent;
  state: RegistryInspectionState;
  onOpenDetail: (detail: RegistryDetailRef) => void | Promise<void>;
}) {
  const { t } = useT();

  return (
    <div className="space-y-4">
      <FingerprintRow
        label={t('registries.detail.fingerprint.configuration')}
        value={record.configurationFingerprint}
      />

      <Section title={t('registries.detail.section.agentLifecycle')}>
        <FieldList
          entries={[
            [t('registries.detail.field.lifecycle'), formatLiteral(record.lifecycleState)],
            [
              t('registries.detail.field.owner'),
              `${formatLiteral(record.ownerActorType)} · ${record.ownerActorId}`,
            ],
            [t('registries.detail.field.created'), formatDate(t, record.createdAt)],
          ]}
        />
        <p className="text-sm text-stone-600 dark:text-neutral-300">
          {t('registries.detail.note.activeRecords')}
        </p>
      </Section>

      <Section title={t('registries.detail.section.exactToolReferences')}>
        <div className="flex flex-wrap gap-2">
          {record.configuration.allowedToolRefs.length === 0 ? (
            <p className="text-sm text-stone-500 dark:text-neutral-400">
              {t('registries.detail.noExactToolRefs')}
            </p>
          ) : (
            record.configuration.allowedToolRefs.map(ref => {
              const resolution = canResolveToolDefinition(state, ref.toolKey, ref.version);
              const label = `${ref.toolKey} v${ref.version}`;
              if (
                resolution.match ||
                resolution.observation === 'not_loaded' ||
                resolution.observation === 'loading'
              ) {
                return (
                  <ReferenceButton
                    key={label}
                    label={label}
                    detail={{ kind: 'tool-definition', key: ref.toolKey, version: ref.version }}
                    onOpenDetail={onOpenDetail}
                  />
                );
              }
              if (
                resolution.observation === 'loaded' ||
                resolution.observation === 'stale' ||
                resolution.observation === 'empty' ||
                resolution.observation === 'blocked'
              ) {
                return <UnresolvedReference key={label} label={label} />;
              }
              return <DeferredReference key={label} label={label} />;
            })
          )}
        </div>
      </Section>

      <Section title={t('registries.detail.section.logicalReferenceWarnings')}>
        <p className="text-sm text-stone-600 dark:text-neutral-300">
          {t('registries.detail.note.logicalReferenceWarnings')}
        </p>
        <FieldList
          entries={[
            [
              t('registries.detail.field.knowledgeScopes'),
              record.configuration.knowledgeScopeRefs.length === 0
                ? t('registries.detail.none')
                : record.configuration.knowledgeScopeRefs
                    .map(ref => `${ref.sourceKey}@${ref.trustVersion} (${ref.accessScope})`)
                    .join(', '),
            ],
            [
              t('registries.detail.field.riskPolicy'),
              record.configuration.riskPolicyRef
                ? `${record.configuration.riskPolicyRef.policyId}@${record.configuration.riskPolicyRef.policyVersion}`
                : t('registries.detail.none'),
            ],
          ]}
        />
      </Section>

      <Section title={t('registries.detail.section.configuration')}>
        <ReadOnlyJson value={record.configuration} />
      </Section>
    </div>
  );
}

function ToolDefinitionDetail({
  record,
  state,
  onOpenDetail,
}: {
  record: ToolRegistryToolDefinition;
  state: RegistryInspectionState;
  onOpenDetail: (detail: RegistryDetailRef) => void | Promise<void>;
}) {
  const { t } = useT();
  const enablement = canResolveToolEnablement(state, record.toolKey, record.version);
  const enablementLabel = enablement.match
    ? enablement.match.lifecycleState === 'enabled'
      ? t('common.enabled')
      : t('common.disabled')
    : t('registries.items.status.noTenantEnablement');

  return (
    <div className="space-y-4">
      <FingerprintRow
        label={t('registries.detail.fingerprint.definition')}
        value={record.definitionFingerprint}
      />

      <Section title={t('registries.detail.section.definitionLifecycle')}>
        <FieldList
          entries={[
            [t('registries.detail.field.lifecycle'), formatLiteral(record.lifecycleState)],
            [t('registries.detail.field.effectClass'), formatLiteral(record.toolEffectClass)],
            [t('registries.detail.field.schemaVersion'), String(record.schemaVersion)],
            [t('registries.detail.field.enablement'), enablementLabel],
          ]}
        />
        <p className="text-sm text-stone-600 dark:text-neutral-300">
          {t('registries.detail.note.activeRecords')}
        </p>
        {enablement.match ? (
          <ReferenceButton
            label={`${enablement.match.toolKey} v${enablement.match.version}`}
            detail={{
              kind: 'tool-enablement',
              key: enablement.match.toolKey,
              version: enablement.match.version,
            }}
            onOpenDetail={onOpenDetail}
          />
        ) : null}
      </Section>

      <Section title={t('registries.detail.section.schemas')}>
        <FieldList
          entries={[
            [t('registries.detail.field.inputSchema'), summarizeSchema(t, record.inputSchema)],
            [t('registries.detail.field.outputSchema'), summarizeSchema(t, record.outputSchema)],
            [
              t('registries.detail.field.timeoutDefaults'),
              summarizeRecordShape(t, record.timeoutDefaults),
            ],
            [
              t('registries.detail.field.retryContract'),
              summarizeRecordShape(t, record.retryContract),
            ],
            [
              t('registries.detail.field.auditContract'),
              summarizeRecordShape(t, record.auditContract),
            ],
          ]}
        />
        <CollapsibleJson
          label={t('registries.detail.viewRawJson')}
          value={{
            inputSchema: record.inputSchema,
            outputSchema: record.outputSchema,
            timeoutDefaults: record.timeoutDefaults,
            retryContract: record.retryContract,
            auditContract: record.auditContract,
          }}
        />
      </Section>
    </div>
  );
}

function ToolEnablementDetail({
  record,
  state,
  onOpenDetail,
}: {
  record: ToolRegistryToolEnablement;
  state: RegistryInspectionState;
  onOpenDetail: (detail: RegistryDetailRef) => void | Promise<void>;
}) {
  const { t } = useT();
  const resolution = canResolveToolDefinition(state, record.toolKey, record.version);

  return (
    <div className="space-y-4">
      <Section title={t('registries.detail.section.enablementLifecycle')}>
        <FieldList
          entries={[
            [t('registries.detail.field.lifecycle'), formatLiteral(record.lifecycleState)],
            [t('registries.detail.field.generation'), String(record.generation)],
            [
              t('registries.detail.field.approvalRequired'),
              record.approvalRequired ? t('common.yes') : t('common.no'),
            ],
            [
              t('registries.detail.field.auditMode'),
              record.auditMode ? formatLiteral(record.auditMode) : t('registries.detail.notSet'),
            ],
            [
              t('registries.detail.field.timeoutCap'),
              record.timeoutCapMs ? `${record.timeoutCapMs} ms` : t('registries.detail.notSet'),
            ],
            [
              t('registries.detail.field.allowTtl'),
              record.allowTtlSeconds ? `${record.allowTtlSeconds}s` : t('registries.detail.notSet'),
            ],
            [t('registries.detail.field.updated'), formatDate(t, record.updatedAt)],
          ]}
        />
        <p className="text-sm text-stone-600 dark:text-neutral-300">
          {t('registries.detail.note.enablementRecords')}
        </p>
      </Section>

      <Section title={t('registries.detail.section.definitionLink')}>
        {resolution.match ? (
          <ReferenceButton
            label={`${record.toolKey} v${record.version}`}
            detail={{ kind: 'tool-definition', key: record.toolKey, version: record.version }}
            onOpenDetail={onOpenDetail}
          />
        ) : resolution.observation === 'loaded' ||
          resolution.observation === 'stale' ||
          resolution.observation === 'empty' ||
          resolution.observation === 'blocked' ? (
          <UnresolvedReference label={`${record.toolKey} v${record.version}`} />
        ) : (
          <DeferredReference label={`${record.toolKey} v${record.version}`} />
        )}
      </Section>
    </div>
  );
}

function ConnectorTypeDetail({ record }: { record: ConnectorRegistryType }) {
  const { t } = useT();

  return (
    <div className="space-y-4">
      <FingerprintRow
        label={t('registries.detail.fingerprint.connectorType')}
        value={record.connectorTypeFingerprint}
      />

      <Section title={t('registries.detail.section.typeLifecycle')}>
        <FieldList
          entries={[
            [t('registries.detail.field.lifecycle'), formatLiteral(record.lifecycleState)],
            [t('registries.detail.field.sourceType'), record.sourceType],
            [
              t('registries.detail.field.capabilities'),
              record.capabilities.join(', ') || t('registries.detail.none'),
            ],
            [t('registries.detail.field.created'), formatDate(t, record.createdAt)],
          ]}
        />
      </Section>

      <Section title={t('registries.detail.section.contracts')}>
        <FieldList
          entries={[
            [
              t('registries.detail.field.normalizationContracts'),
              summarizeNormalizationContracts(t, record.normalizationContracts),
            ],
            [
              t('registries.detail.field.deliveryBehavior'),
              summarizeScalarRecord(t, record.deliveryBehavior),
            ],
          ]}
        />
        <CollapsibleJson
          label={t('registries.detail.viewRawJson')}
          value={{
            normalizationContracts: record.normalizationContracts,
            deliveryBehavior: record.deliveryBehavior,
          }}
        />
      </Section>
    </div>
  );
}

function ConnectorBindingDetail({
  record,
  state,
  onOpenDetail,
}: {
  record: ConnectorRegistryBinding;
  state: RegistryInspectionState;
  onOpenDetail: (detail: RegistryDetailRef) => void | Promise<void>;
}) {
  const { t } = useT();
  const resolution = canResolveConnectorType(
    state,
    record.connectorTypeKey,
    record.connectorTypeVersion
  );

  return (
    <div className="space-y-4">
      <FingerprintRow
        label={t('registries.detail.fingerprint.binding')}
        value={record.bindingFingerprint}
      />

      <Section title={t('registries.detail.section.bindingLifecycle')}>
        <FieldList
          entries={[
            [t('registries.detail.field.lifecycle'), formatLiteral(record.lifecycleState)],
            [
              t('registries.detail.field.providerAccountReference'),
              `${record.providerAccount.namespace}:${record.providerAccount.externalAccountRef}`,
            ],
            [
              t('registries.detail.field.capabilities'),
              record.enabledCapabilities.join(', ') || t('registries.detail.none'),
            ],
            [t('registries.detail.field.created'), formatDate(t, record.createdAt)],
          ]}
        />
      </Section>

      <Section title={t('registries.detail.section.exactConnectorType')}>
        {resolution.match ? (
          <ReferenceButton
            label={`${record.connectorTypeKey} v${record.connectorTypeVersion}`}
            detail={{
              kind: 'connector-type',
              key: record.connectorTypeKey,
              version: record.connectorTypeVersion,
            }}
            onOpenDetail={onOpenDetail}
          />
        ) : resolution.observation === 'loaded' ||
          resolution.observation === 'stale' ||
          resolution.observation === 'empty' ||
          resolution.observation === 'blocked' ? (
          <UnresolvedReference
            label={`${record.connectorTypeKey} v${record.connectorTypeVersion}`}
          />
        ) : (
          <DeferredReference label={`${record.connectorTypeKey} v${record.connectorTypeVersion}`} />
        )}
      </Section>

      <Section title={t('registries.detail.section.logicalReferences')}>
        <p className="text-sm text-stone-600 dark:text-neutral-300">
          {t('registries.detail.note.logicalReferencesSecret')}
        </p>
        <FieldList
          entries={[
            [t('registries.detail.field.configReference'), record.configRef],
            [t('registries.detail.field.credentialReference'), record.credentialRef],
          ]}
        />
      </Section>
    </div>
  );
}

export default function RegistryDetailPane({
  activeTab: _activeTab,
  detailState,
  state,
  onOpenDetail,
}: RegistryDetailPaneProps) {
  const { t } = useT();

  if (detailState.kind === 'none') {
    return (
      <div className="rounded-3xl border border-dashed border-stone-300 bg-stone-50 px-5 py-6 text-sm text-stone-500 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-400">
        {t('registries.detail.empty')}
      </div>
    );
  }

  if (detailState.kind === 'loading') {
    return (
      <div className="rounded-3xl border border-stone-200 bg-white px-5 py-6 text-sm text-stone-500 dark:border-neutral-800 dark:bg-neutral-900 dark:text-neutral-400">
        {t('registries.detail.state.loading')}
      </div>
    );
  }

  if (detailState.kind === 'missing') {
    return (
      <div className="rounded-3xl border border-amber-200 bg-amber-50 px-5 py-6 text-sm text-amber-800">
        {t('registries.detail.state.missing')}
      </div>
    );
  }

  if (detailState.kind === 'error') {
    return (
      <div className="rounded-3xl border border-coral-200 bg-coral-50 px-5 py-6 text-sm text-coral-800">
        {t('registries.detail.state.error')}
      </div>
    );
  }

  const { detail, record } = detailState;
  const title = `${detail.key} v${detail.version}`;

  return (
    <div className="space-y-4">
      <header>
        <p className="text-[11px] font-semibold uppercase tracking-[0.24em] text-stone-500 dark:text-neutral-400">
          {t(`registries.detail.kind.${detail.kind}`)}
        </p>
        <h3 className="mt-1 text-xl font-semibold text-stone-900 dark:text-neutral-100">{title}</h3>
      </header>

      {detail.kind === 'agent' ? (
        <AgentDetail
          record={record as AgentRegistryAgent}
          state={state}
          onOpenDetail={onOpenDetail}
        />
      ) : null}
      {detail.kind === 'tool-definition' ? (
        <ToolDefinitionDetail
          record={record as ToolRegistryToolDefinition}
          state={state}
          onOpenDetail={onOpenDetail}
        />
      ) : null}
      {detail.kind === 'tool-enablement' ? (
        <ToolEnablementDetail
          record={record as ToolRegistryToolEnablement}
          state={state}
          onOpenDetail={onOpenDetail}
        />
      ) : null}
      {detail.kind === 'connector-type' ? (
        <ConnectorTypeDetail record={record as ConnectorRegistryType} />
      ) : null}
      {detail.kind === 'connector-binding' ? (
        <ConnectorBindingDetail
          record={record as ConnectorRegistryBinding}
          state={state}
          onOpenDetail={onOpenDetail}
        />
      ) : null}
    </div>
  );
}
