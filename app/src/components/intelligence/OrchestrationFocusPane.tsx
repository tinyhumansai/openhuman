/**
 * OrchestrationFocusPane — the right-hand focus column of the TinyPlace
 * Orchestration tab: the selected chat's header, the subconscious steering
 * status header, the message transcript (with load/error/empty states), and the
 * Master/session composer. Presentational: all state + handlers come from the
 * tab container.
 */
import type { FormEvent, ReactElement } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import type { useOrchestrationChats } from '../../lib/orchestration/useOrchestrationChats';
import Button from '../ui/Button';
import { MessageBubble } from './OrchestrationChatPrimitives';

type ChatsApi = ReturnType<typeof useOrchestrationChats>;

export interface OrchestrationFocusPaneProps {
  selected: ChatsApi['selected'];
  sessionsState: ChatsApi['sessionsState'];
  messagesState: ChatsApi['messagesState'];
  status: ChatsApi['status'];
  masterError: ChatsApi['masterError'];
  refresh: ChatsApi['refresh'];
  steeringText: string | null;
  runningReview: boolean;
  onRunSteeringReview: () => void;
  canCompose: boolean;
  composerBody: string;
  onComposerChange: (value: string) => void;
  sending: boolean;
  onSubmitComposer: (event: FormEvent<HTMLFormElement>) => void;
}

export default function OrchestrationFocusPane({
  selected,
  sessionsState,
  messagesState,
  status,
  masterError,
  refresh,
  steeringText,
  runningReview,
  onRunSteeringReview,
  canCompose,
  composerBody,
  onComposerChange,
  sending,
  onSubmitComposer,
}: OrchestrationFocusPaneProps): ReactElement {
  const { t } = useT();
  return (
    <main className="flex min-w-0 flex-1 flex-col bg-surface">
      <div className="flex items-center justify-between gap-3 border-b border-line px-5 py-4">
        <div className="min-w-0">
          <h3 className="truncate text-base font-semibold text-content">{selected?.title}</h3>
          <p className="mt-0.5 truncate text-xs text-content-muted">{selected?.subtitle}</p>
        </div>
        {selected && !selected.pinned ? (
          <span
            className={`rounded-full px-2 py-1 text-xs font-medium ${
              selected.active
                ? 'bg-sage-100 text-sage-700 dark:bg-sage-500/15 dark:text-sage-300'
                : 'bg-surface-strong text-content-muted'
            }`}>
            {selected.active
              ? t('tinyplaceOrchestration.active')
              : t('tinyplaceOrchestration.inactive')}
          </span>
        ) : null}
      </div>

      {/* Steering status header — the tinyplace subconscious instance's output. */}
      {selected?.kind === 'subconscious' ? (
        <div
          data-testid="tinyplace-steering-header"
          className="flex items-center justify-between gap-3 border-b border-line bg-amber-50/40 px-5 py-3 dark:bg-amber-500/5">
          <div className="min-w-0">
            <p className="text-xs font-medium text-content">
              {steeringText
                ? t('tinyplaceOrchestration.steeringHeader.current')
                : t('tinyplaceOrchestration.steeringHeader.none')}
            </p>
            {steeringText ? (
              <p className="mt-0.5 truncate text-xs text-content-muted">{steeringText}</p>
            ) : null}
            <p className="mt-0.5 text-[11px] text-content-faint">
              {status?.steering
                ? t('tinyplaceOrchestration.steeringHeader.expires').replace(
                    '{n}',
                    String(status.steering.expiresAfterCycles)
                  )
                : ''}
              {status?.lastTickAt
                ? `${status?.steering ? ' · ' : ''}${t(
                    'tinyplaceOrchestration.steeringHeader.lastReview'
                  )}: ${new Date(status.lastTickAt * 1000).toLocaleTimeString()}`
                : ''}
            </p>
          </div>
          <Button
            variant="secondary"
            size="sm"
            onClick={() => void onRunSteeringReview()}
            disabled={runningReview}>
            {runningReview
              ? t('tinyplaceOrchestration.steeringHeader.running')
              : t('tinyplaceOrchestration.steeringHeader.runReview')}
          </Button>
        </div>
      ) : null}

      {sessionsState.status === 'loading' ? (
        <div className="flex flex-1 items-center justify-center text-sm text-content-muted">
          {t('tinyplaceOrchestration.loading')}
        </div>
      ) : sessionsState.status === 'payment_required' ? (
        <div className="flex flex-1 items-center justify-center text-sm text-amber-600 dark:text-amber-300">
          {t('tinyplaceOrchestration.paymentRequired')}
        </div>
      ) : sessionsState.status === 'error' ? (
        <div className="flex flex-1 flex-col items-center justify-center gap-3 text-sm text-coral-600 dark:text-coral-300">
          <p>
            {t('tinyplaceOrchestration.failedToLoad')}: {sessionsState.message}
          </p>
          <Button variant="secondary" size="sm" onClick={() => void refresh()}>
            {t('common.retry')}
          </Button>
        </div>
      ) : messagesState.status === 'loading' ? (
        <div className="flex flex-1 items-center justify-center text-sm text-content-muted">
          {t('tinyplaceOrchestration.loading')}
        </div>
      ) : messagesState.status === 'error' ? (
        <div className="flex flex-1 flex-col items-center justify-center gap-3 text-sm text-coral-600 dark:text-coral-300">
          <p>
            {t('tinyplaceOrchestration.failedToLoad')}: {messagesState.message}
          </p>
          <Button variant="secondary" size="sm" onClick={() => void refresh()}>
            {t('common.retry')}
          </Button>
        </div>
      ) : selected?.messages.length ? (
        <div className="min-h-0 flex-1 overflow-y-auto bg-surface-muted/20 p-5">
          <div className="space-y-3" data-testid="tinyplace-chat-messages">
            {selected.messages.map(message => (
              <MessageBubble key={message.id} message={message} />
            ))}
          </div>
        </div>
      ) : (
        <div className="flex flex-1 items-center justify-center px-6 text-center text-sm text-content-faint">
          {t('tinyplaceOrchestration.noMessages')}
        </div>
      )}

      {canCompose && sessionsState.status === 'ok' ? (
        <form
          className="flex flex-col gap-2 border-t border-line px-5 py-3"
          onSubmit={onSubmitComposer}>
          {masterError ? (
            <p className="rounded-md bg-coral-50 px-2 py-1 text-xs text-coral-700 dark:bg-coral-500/10 dark:text-coral-300">
              {t('tinyplaceOrchestration.composer.sendFailed')}: {masterError}
            </p>
          ) : null}
          <div className="flex gap-2">
            <input
              data-testid="tinyplace-master-composer-input"
              value={composerBody}
              onChange={event => onComposerChange(event.target.value)}
              placeholder={t('tinyplaceOrchestration.composer.placeholder')}
              className="min-w-0 flex-1 rounded-md border border-line bg-surface px-3 py-2 text-sm text-content outline-none transition focus:border-ocean-500 focus:ring-2 focus:ring-ocean-500/20"
            />
            <Button
              type="submit"
              variant="primary"
              size="sm"
              data-testid="tinyplace-master-composer-send"
              disabled={!composerBody.trim() || sending}>
              {t('tinyplaceOrchestration.composer.send')}
            </Button>
          </div>
        </form>
      ) : null}
    </main>
  );
}
