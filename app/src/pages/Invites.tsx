import debugFactory from 'debug';
import { useEffect, useMemo, useRef, useState } from 'react';

import InviteProgressBar from '../components/share/InviteProgressBar';
import SocialShareRow from '../components/share/SocialShareRow';
import ViralInviteCard from '../components/share/ViralInviteCard';
import { useUser } from '../hooks/useUser';
import { inviteApi } from '../services/api/inviteApi';
import type { InviteCode } from '../types/invite';
import { buildInviteUrl, copyToClipboard, defaultInviteMessage } from '../utils/share';

const log = debugFactory('invites');

type RedeemStatus = 'idle' | 'loading' | 'success' | 'error';

function CodeRow({ invite }: { invite: InviteCode }) {
  const [copied, setCopied] = useState(false);
  const url = buildInviteUrl(invite.code);
  const claimed = invite.currentUses >= invite.maxUses;
  const claimedUser = invite.usageHistory[0]?.userId;
  const displayName = claimedUser?.username
    ? `@${claimedUser.username}`
    : claimedUser?.firstName || 'a friend';

  const handleCopy = async () => {
    const ok = await copyToClipboard(url);
    if (ok) {
      setCopied(true);
      setTimeout(() => setCopied(false), 1800);
    }
  };

  return (
    <div className="flex items-center justify-between gap-3 rounded-xl border border-stone-200 bg-white/60 px-4 py-3 transition-colors hover:bg-white">
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="font-mono text-sm font-semibold tracking-[0.25em] text-stone-900">
            {invite.code}
          </span>
          {claimed ? (
            <span className="rounded-full bg-sage-100 px-2 py-0.5 text-[10px] font-medium text-sage-700">
              Joined
            </span>
          ) : (
            <span className="rounded-full bg-amber-50 px-2 py-0.5 text-[10px] font-medium text-amber-700">
              Unused
            </span>
          )}
        </div>
        <p className="mt-0.5 truncate text-[11px] text-stone-500">
          {claimed ? `Redeemed by ${displayName}` : url.replace(/^https?:\/\//, '')}
        </p>
      </div>
      {!claimed ? (
        <button
          type="button"
          onClick={() => void handleCopy()}
          className={`whitespace-nowrap rounded-full px-3 py-1.5 text-[11px] font-medium transition-colors ${
            copied ? 'bg-sage-500 text-white' : 'bg-stone-900 text-white hover:bg-stone-800'
          }`}>
          {copied ? 'Link copied' : 'Copy link'}
        </button>
      ) : (
        <span className="text-[11px] text-stone-400">Thanks for sharing!</span>
      )}
    </div>
  );
}

const Invites = () => {
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

  const availableCode = useMemo(
    () => codes.find(c => c.currentUses < c.maxUses) ?? codes[0],
    [codes]
  );

  const convertedCount = useMemo(
    () => codes.filter(c => c.currentUses >= c.maxUses).length,
    [codes]
  );

  const heroCode = availableCode?.code ?? '';
  const heroUrl = useMemo(() => buildInviteUrl(heroCode), [heroCode]);
  const heroMessage = useMemo(
    () => defaultInviteMessage(heroCode || 'OPENHUMAN', heroUrl),
    [heroCode, heroUrl]
  );

  return (
    <div className="min-h-full p-4 pt-6 pb-10">
      <div className="mx-auto max-w-xl space-y-4">
        {/* Hero — shareable invite card */}
        {heroCode ? (
          <div className="animate-fade-up space-y-3">
            <ViralInviteCard
              code={heroCode}
              url={heroUrl}
              firstName={user?.firstName ?? undefined}
            />
            <div className="rounded-2xl border border-stone-200 bg-white p-4 shadow-soft">
              <p className="mb-2 text-[11px] font-medium uppercase tracking-wider text-stone-400">
                Share with one tap
              </p>
              <SocialShareRow url={heroUrl} message={heroMessage} variant="spacious" />
            </div>
          </div>
        ) : null}

        {/* Redeem Section — shown only if user hasn't redeemed yet */}
        {!hasBeenInvited && (
          <div className="animate-fade-up rounded-2xl border border-stone-200 bg-white p-6 shadow-soft">
            <h2 className="mb-1 text-lg font-bold">Got a code from a friend?</h2>
            <p className="mb-4 text-xs text-stone-500">
              Enter it below to unlock free credits — welcome to the inner circle.
            </p>
            <div className="flex gap-2">
              <input
                type="text"
                value={redeemInput}
                onChange={e => setRedeemInput(e.target.value.toUpperCase())}
                onKeyDown={e => e.key === 'Enter' && void handleRedeem()}
                placeholder="Enter code"
                className="flex-1 rounded-xl border border-stone-200 bg-white px-4 py-2.5 font-mono text-sm tracking-wider text-stone-900 placeholder:font-sans placeholder:tracking-normal placeholder:text-stone-400 focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/40"
                disabled={redeemStatus === 'loading'}
              />
              <button
                onClick={() => void handleRedeem()}
                disabled={redeemStatus === 'loading' || !redeemInput.trim()}
                className="whitespace-nowrap rounded-xl bg-primary-500 px-5 py-2.5 text-sm font-medium text-white transition-colors hover:bg-primary-600 disabled:cursor-not-allowed disabled:opacity-50">
                {redeemStatus === 'loading' ? '...' : 'Redeem'}
              </button>
            </div>
            {redeemStatus === 'success' && (
              <p className="mt-2 text-xs text-sage-600">Invite code redeemed successfully!</p>
            )}
            {redeemStatus === 'error' && redeemError && (
              <p className="mt-2 text-xs text-coral-500">{redeemError}</p>
            )}
          </div>
        )}

        {/* Your Invite Codes */}
        <div className="animate-fade-up rounded-2xl border border-stone-200 bg-white p-6 shadow-soft">
          <div className="mb-4 flex items-start justify-between gap-3">
            <div>
              <h2 className="text-lg font-bold">Your invite codes</h2>
              <p className="text-xs text-stone-500">
                Each code is one magic seat. They go fast — share today.
              </p>
            </div>
          </div>

          {codes.length > 0 ? (
            <div className="mb-4">
              <InviteProgressBar converted={convertedCount} total={codes.length} />
            </div>
          ) : null}

          {loadError && <p className="text-center text-xs text-coral-500">{loadError}</p>}

          {isLoading ? (
            <div className="space-y-3">
              {Array.from({ length: 5 }).map((_, i) => (
                <div key={i} className="h-12 animate-pulse rounded-xl bg-stone-100" />
              ))}
            </div>
          ) : codes.length > 0 ? (
            <div className="space-y-2">
              {codes.map(invite => (
                <CodeRow key={invite._id} invite={invite} />
              ))}
            </div>
          ) : (
            <p className="py-6 text-center text-sm text-stone-500">
              No invite codes available yet.
            </p>
          )}
        </div>

        {/* Social proof / trust row */}
        <div className="animate-fade-up rounded-2xl border border-stone-200 bg-gradient-to-br from-primary-50 via-white to-sage-50 p-5 shadow-soft">
          <div className="flex items-center gap-3">
            <div className="flex -space-x-2">
              <span className="h-8 w-8 rounded-full border-2 border-white bg-primary-500" />
              <span className="h-8 w-8 rounded-full border-2 border-white bg-sage-500" />
              <span className="h-8 w-8 rounded-full border-2 border-white bg-amber-500" />
              <span className="flex h-8 w-8 items-center justify-center rounded-full border-2 border-white bg-stone-900 text-[10px] font-semibold text-white">
                +
              </span>
            </div>
            <div>
              <p className="text-sm font-semibold text-stone-900">
                Join builders shipping with OpenHuman
              </p>
              <p className="text-xs text-stone-500">
                Every invite you send helps more humans reclaim their time.
              </p>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
};

export default Invites;
