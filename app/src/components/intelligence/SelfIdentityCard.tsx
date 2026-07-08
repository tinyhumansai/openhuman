/**
 * SelfIdentityCard — pinned at the top of the roster, shows OpenHuman's *own*
 * tiny.place identity so the user can hand it to a peer to be messaged inbound.
 *
 * Why it exists: the tab previously showed peers/contacts but never OpenHuman's
 * own agent id, so there was no way to say "here's my address, DM me". Worse, a
 * fresh identity can accept contacts yet stay un-messageable until it registers a
 * @handle (which is what publishes its directory card + Signal key) — every send
 * to it 404s on `/directory/agents/<addr>`. This card surfaces both the address
 * and the discoverability gap instead of leaving it a mystery.
 *
 * Presentational only — the parent fetches {@link SelfIdentity} via
 * `orchestrationClient.selfIdentity()` and owns refresh.
 */
import { useCallback, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import type { SelfIdentity } from '../../lib/orchestration/orchestrationClient';

export interface SelfIdentityCardProps {
  identity: SelfIdentity | null;
  loading: boolean;
  /**
   * Publish (or refresh) this agent's directory card + Signal key so peers can
   * DM it. Rendered only while undiscoverable. The parent owns the RPC + the
   * identity refresh; the card just drives the button state.
   */
  onPublish?: () => void;
  /** True while {@link onPublish} is in flight — disables the button. */
  publishing?: boolean;
  /** Non-null when the last publish attempt failed — surfaces under the button. */
  publishError?: string | null;
}

function shortAddress(address: string): string {
  if (address.length <= 14) return address;
  return `${address.slice(0, 6)}…${address.slice(-5)}`;
}

export default function SelfIdentityCard({
  identity,
  loading,
  onPublish,
  publishing,
  publishError,
}: SelfIdentityCardProps): React.ReactElement {
  const { t } = useT();
  const [copied, setCopied] = useState(false);

  const address = identity?.agentId ?? '';
  const onCopy = useCallback(() => {
    if (!address) return;
    void navigator.clipboard?.writeText(address).then(
      () => {
        setCopied(true);
        window.setTimeout(() => setCopied(false), 1500);
      },
      () => setCopied(false)
    );
  }, [address]);

  if (loading && !identity) {
    return (
      <section
        data-testid="tinyplace-self-identity"
        className="border-b border-line bg-surface-muted/40 px-4 py-3 text-[11px] text-content-faint">
        {t('tinyplaceOrchestration.identity.loading')}
      </section>
    );
  }

  if (!identity) return <></>;

  const primaryHandle = identity.primaryHandle ?? identity.handles[0]?.username;

  return (
    <section
      data-testid="tinyplace-self-identity"
      className="border-b border-line bg-surface-muted/40 px-4 py-3">
      <div className="flex items-center gap-2">
        <span
          aria-hidden
          className="flex h-6 w-6 flex-none items-center justify-center rounded-md bg-sage-500 font-mono text-[11px] font-bold text-white">
          OH
        </span>
        <div className="min-w-0 flex-1">
          <div className="truncate text-xs font-semibold text-content">
            {primaryHandle ? `@${primaryHandle}` : t('tinyplaceOrchestration.identity.noHandle')}
          </div>
          <div className="truncate font-mono text-[10px] text-content-faint" title={address}>
            {shortAddress(address)}
          </div>
        </div>
        <button
          type="button"
          data-testid="tinyplace-self-identity-copy"
          onClick={onCopy}
          disabled={!address}
          className="flex-none rounded-md px-1.5 py-0.5 text-[11px] font-semibold text-ocean-600 transition hover:bg-ocean-500/10 disabled:opacity-40 dark:text-ocean-300">
          {copied
            ? t('tinyplaceOrchestration.identity.copied')
            : t('tinyplaceOrchestration.identity.copy')}
        </button>
      </div>

      <div className="mt-2 flex flex-wrap items-center gap-1.5">
        {identity.discoverable ? (
          <span
            data-testid="tinyplace-self-identity-status"
            data-discoverable="true"
            className="inline-flex items-center rounded-full bg-sage-500/10 px-2 py-0.5 text-[10px] font-semibold text-sage-700 dark:text-sage-300">
            {t('tinyplaceOrchestration.identity.discoverable')}
          </span>
        ) : (
          <span
            data-testid="tinyplace-self-identity-status"
            data-discoverable="false"
            className="inline-flex items-center rounded-full bg-coral-500/10 px-2 py-0.5 text-[10px] font-semibold text-coral-700 dark:text-coral-300">
            {t('tinyplaceOrchestration.identity.undiscoverable')}
          </span>
        )}
        <span
          className="inline-flex items-center gap-1 rounded-full bg-surface-strong px-2 py-0.5 text-[10px] text-content-muted"
          title={t('tinyplaceOrchestration.identity.card')}>
          {t('tinyplaceOrchestration.identity.card')}:{' '}
          {identity.cardPublished
            ? t('tinyplaceOrchestration.identity.published')
            : t('tinyplaceOrchestration.identity.notPublished')}
        </span>
        <span
          className="inline-flex items-center gap-1 rounded-full bg-surface-strong px-2 py-0.5 text-[10px] text-content-muted"
          title={t('tinyplaceOrchestration.identity.key')}>
          {t('tinyplaceOrchestration.identity.key')}:{' '}
          {identity.keyPublished
            ? t('tinyplaceOrchestration.identity.published')
            : t('tinyplaceOrchestration.identity.notPublished')}
        </span>
      </div>

      {identity.discoverable && onPublish ? (
        <div className="mt-2 flex flex-wrap items-center gap-2">
          <button
            type="button"
            data-testid="tinyplace-self-identity-republish"
            onClick={onPublish}
            disabled={publishing}
            className="inline-flex items-center rounded-md border border-line px-2 py-0.5 text-[10px] font-medium text-content-muted transition hover:bg-surface-hover disabled:opacity-50">
            {publishing
              ? t('tinyplaceOrchestration.identity.publishing')
              : t('tinyplaceOrchestration.identity.republish')}
          </button>
          {publishError ? (
            <span
              data-testid="tinyplace-self-identity-republish-error"
              className="text-[10px] text-coral-600 dark:text-coral-300">
              {t('tinyplaceOrchestration.identity.publishFailed')}
            </span>
          ) : null}
        </div>
      ) : null}

      {!identity.discoverable ? (
        <div className="mt-2 rounded-md bg-coral-50 px-2 py-1.5 dark:bg-coral-500/10">
          <p className="text-[10px] text-coral-700 dark:text-coral-300">
            {t('tinyplaceOrchestration.identity.undiscoverableHint')}
          </p>
          {onPublish ? (
            <button
              type="button"
              data-testid="tinyplace-self-identity-publish"
              onClick={onPublish}
              disabled={publishing}
              className="mt-1.5 inline-flex items-center rounded-md bg-coral-600 px-2 py-1 text-[11px] font-semibold text-white transition hover:bg-coral-700 disabled:opacity-50 dark:bg-coral-500 dark:hover:bg-coral-400">
              {publishing
                ? t('tinyplaceOrchestration.identity.publishing')
                : t('tinyplaceOrchestration.identity.makeDiscoverable')}
            </button>
          ) : null}
          {publishError ? (
            <p
              data-testid="tinyplace-self-identity-publish-error"
              className="mt-1 text-[10px] text-coral-700 dark:text-coral-300">
              {t('tinyplaceOrchestration.identity.publishFailed')}
            </p>
          ) : null}
        </div>
      ) : null}
    </section>
  );
}
