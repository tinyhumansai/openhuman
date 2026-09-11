import debugFactory from 'debug';
import { useEffect, useRef, useState } from 'react';

import PageSectionHeader from '../components/layout/PageSectionHeader';
import { Card, CenteredLoadingState, EmptyState } from '../components/ui';
import Button from '../components/ui/Button';
import { useClipboardFeedback } from '../hooks/useClipboardFeedback';
import { useUser } from '../hooks/useUser';
import { useT } from '../lib/i18n/I18nContext';
import { inviteApi } from '../services/api/inviteApi';
import type { InviteCode } from '../types/invite';

const log = debugFactory('invites');

type RedeemStatus = 'idle' | 'loading' | 'success' | 'error';

const CodeRow = ({ invite }: { invite: InviteCode }) => {
  const { t } = useT();
  const clipboard = useClipboardFeedback();
  const claimed = invite.currentUses >= invite.maxUses;
  const claimedUser = invite.usageHistory[0]?.userId;

  const displayName = claimedUser?.username
    ? `@${claimedUser.username}`
    : claimedUser?.firstName || 'Someone';

  return (
    <div className="flex items-center justify-between py-3 px-4 rounded-xl bg-surface/5 hover:bg-surface-hover transition-colors">
      <div className="flex-1 min-w-0">
        <span className="font-mono text-sm tracking-wider">{invite.code}</span>
        {claimed && (
          <p className="text-xs text-content-muted mt-0.5">
            {t('rewards.credits')} {displayName}
          </p>
        )}
      </div>
      <div className="flex items-center gap-2 ml-3">
        {claimed ? (
          <span className="text-xs px-2 py-1 rounded-full bg-surface-strong text-content-faint">
            {t('common.disabled')}
          </span>
        ) : (
          <span className="text-xs px-2 py-1 rounded-full bg-sage-500/20 text-sage-500">
            {t('common.enabled')}
          </span>
        )}
        <Button
          iconOnly
          variant="tertiary"
          size="sm"
          onClick={() => void clipboard.copy(invite.code)}
          aria-label={t('common.copy')}
          title={t('common.copy')}>
          {clipboard.status === 'copied' ? (
            <svg
              className="w-4 h-4 text-sage-500"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M5 13l4 4L19 7"
              />
            </svg>
          ) : (
            <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M8 16H6a2 2 0 01-2-2V6a2 2 0 012-2h8a2 2 0 012 2v2m-6 12h8a2 2 0 002-2v-8a2 2 0 00-2-2h-8a2 2 0 00-2 2v8a2 2 0 002 2z"
              />
            </svg>
          )}
        </Button>
      </div>
    </div>
  );
};

const Invites = () => {
  const { t } = useT();
  const { user, refetch: refetchUser } = useUser();
  const [codes, setCodes] = useState<InviteCode[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [redeemStatus, setRedeemStatus] = useState<RedeemStatus>('idle');
  const [redeemError, setRedeemError] = useState<string | null>(null);

  const [redeemInput, setRedeemInput] = useState('');
  const redeemTimeoutRef = useRef<number | null>(null);
  const loadRequestIdRef = useRef(0);
  const hasBeenInvited = !!user?.referral?.invitedBy;

  const [loadError, setLoadError] = useState<string | null>(null);

  const loadInviteCodes = async () => {
    const requestId = ++loadRequestIdRef.current;
    setIsLoading(true);
    setLoadError(null);
    try {
      const data = await inviteApi.getMyInviteCodes();
      if (requestId !== loadRequestIdRef.current) return;
      setCodes(data);
    } catch (error) {
      if (requestId !== loadRequestIdRef.current) return;
      log('loadInviteCodes failed requestId=%d error=%O', requestId, error);
      setLoadError(error instanceof Error ? error.message : 'Failed to load invite codes');
    } finally {
      if (requestId === loadRequestIdRef.current) {
        setIsLoading(false);
      }
    }
  };

  useEffect(() => {
    void loadInviteCodes();
    return () => {
      // Invalidate any in-flight loadInviteCodes requests
      loadRequestIdRef.current += 1;
      if (redeemTimeoutRef.current) {
        clearTimeout(redeemTimeoutRef.current);
        redeemTimeoutRef.current = null;
      }
    };
  }, []);

  const handleRedeem = async () => {
    const trimmed = redeemInput.trim();
    if (!trimmed) return;

    setRedeemStatus('loading');
    setRedeemError(null);

    try {
      await inviteApi.redeemInviteCode(trimmed);
      await loadInviteCodes();
      setRedeemInput('');
      setRedeemStatus('success');
      if (redeemTimeoutRef.current) {
        clearTimeout(redeemTimeoutRef.current);
      }
      redeemTimeoutRef.current = window.setTimeout(() => {
        redeemTimeoutRef.current = null;
        setRedeemStatus('idle');
        setRedeemError(null);
      }, 3000);
      // Refresh user in background — don't let failure override the successful redeem
      refetchUser().catch(() => {});
    } catch (error) {
      setRedeemStatus('error');
      setRedeemError(error instanceof Error ? error.message : 'Failed to redeem invite code');
    }
  };

  return (
    // The page column follows `Notifications.tsx`: an outer `space-y-4 p-4 pt-6`
    // gutter, a `PageSectionHeader` first, and `mx-auto max-w-lg` repeated on
    // each section below it — `PageSectionHeader`'s own `width` caps only the
    // header card, not the body. `max-w-lg` is the contentWidth `sm` step; the
    // hand-picked `max-w-md` this replaced is on no scale.
    <div className="min-h-full space-y-4 p-4 pt-6">
      <PageSectionHeader className="mx-auto max-w-lg" title={t('invites.title')} />

      {/* Redeem Section — shown only if user hasn't redeemed yet */}
      {!hasBeenInvited && (
        // Card imposes a divide-y body and a shrunk h3 title (see
        // Card.tsx), neither wanted here, so the whole block stays one
        // child and this heading keeps its own markup — only the
        // bordered-surface recipe (now rounded-xl, Card's radius)
        // moves onto the primitive.
        <Card divided={false} className="mx-auto max-w-lg shadow-soft animate-fade-up p-6">
          <h2 className="text-lg font-bold mb-1">{t('invites.redeemHeading')}</h2>
          <p className="text-xs opacity-70 mb-4">{t('rewards.share')}</p>
          <div className="flex gap-2">
            <input
              type="text"
              value={redeemInput}
              onChange={e => setRedeemInput(e.target.value.toUpperCase())}
              onKeyDown={e => e.key === 'Enter' && handleRedeem()}
              placeholder={t('invites.redeemPlaceholder')}
              className="flex-1 px-4 py-2.5 bg-surface/5 border border-line-strong rounded-xl font-mono text-sm tracking-wider placeholder:text-content-faint placeholder:tracking-normal placeholder:font-sans focus:outline-hidden focus:ring-2 focus:ring-primary-500/50 focus:border-primary-500/50 transition-all"
              disabled={redeemStatus === 'loading'}
            />
            <Button
              variant="primary"
              onClick={handleRedeem}
              disabled={redeemStatus === 'loading' || !redeemInput.trim()}
              className="whitespace-nowrap">
              {redeemStatus === 'loading' ? '...' : t('invites.redeemSubmit')}
            </Button>
          </div>
          {redeemStatus === 'success' && (
            <p className="text-sage-500 text-xs mt-2">{t('common.success')}</p>
          )}
          {redeemStatus === 'error' && redeemError && (
            <p className="text-coral-500 text-xs mt-2">{redeemError}</p>
          )}
        </Card>
      )}

      {/* Your Invite Codes */}
      <Card divided={false} className="mx-auto max-w-lg shadow-soft animate-fade-up p-6">
        <div className="mb-4">
          <h2 className="text-lg font-bold mb-1">{t('rewards.referralCode')}</h2>
          <p className="text-xs opacity-70">{t('rewards.share')}</p>
        </div>

        {loadError && <p className="text-coral-500 text-xs text-center py-2">{loadError}</p>}

        {isLoading ? (
          <CenteredLoadingState label={t('common.loading')} />
        ) : codes.length > 0 ? (
          <div className="space-y-2">
            {codes.map(invite => (
              <CodeRow key={invite._id} invite={invite} />
            ))}
          </div>
        ) : (
          <EmptyState label={t('invites.noInvites')} className="text-center" />
        )}
      </Card>
    </div>
  );
};

export default Invites;
