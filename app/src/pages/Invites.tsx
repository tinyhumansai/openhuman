import debugFactory from 'debug';
import { useEffect, useRef, useState } from 'react';

import { useUser } from '../hooks/useUser';
import { inviteApi } from '../services/api/inviteApi';
import type { InviteCode } from '../types/invite';

const log = debugFactory('invites');

type RedeemStatus = 'idle' | 'loading' | 'success' | 'error';

const CodeRow = ({ invite }: { invite: InviteCode }) => {
  const [copied, setCopied] = useState(false);
  const claimed = invite.currentUses >= invite.maxUses;
  const claimedUser = invite.usageHistory[0]?.userId;

  const handleCopy = async () => {
    await navigator.clipboard.writeText(invite.code);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  const displayName = claimedUser?.username
    ? `@${claimedUser.username}`
    : claimedUser?.firstName || 'Someone';

  return (
    <div className="flex items-center justify-between py-3 px-4 rounded-xl bg-white/5 hover:bg-white/[0.07] transition-colors">
      <div className="flex-1 min-w-0">
        <span className="font-mono text-sm tracking-wider">{invite.code}</span>
        {claimed && <p className="text-xs text-stone-500 mt-0.5">Claimed by {displayName}</p>}
      </div>
      <div className="flex items-center gap-2 ml-3">
        {claimed ? (
          <span className="text-xs px-2 py-1 rounded-full bg-stone-700/50 text-stone-400">
            Used
          </span>
        ) : (
          <span className="text-xs px-2 py-1 rounded-full bg-sage-500/20 text-sage-500">
            Available
          </span>
        )}
        <button
          onClick={handleCopy}
          className="p-1.5 rounded-lg hover:bg-white/10 transition-colors text-stone-400 hover:text-stone-200"
          title="Copy code">
          {copied ? (
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
        </button>
      </div>
    </div>
  );
};

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

  return (
    <div className="min-h-full flex items-center justify-center p-4 pt-6">
      <div className="max-w-md w-full space-y-4">
        <div>
          <div className="space-y-4">
            {/* Redeem Section — shown only if user hasn't redeemed yet */}
            {!hasBeenInvited && (
              <div className="bg-white rounded-2xl shadow-soft border border-stone-200 p-6 animate-fade-up">
                <h2 className="text-lg font-bold mb-1">Redeem an Invite Code</h2>
                <p className="text-xs opacity-70 mb-4">
                  Got a code from a friend? Enter it below to unlock free credits.
                </p>
                <div className="flex gap-2">
                  <input
                    type="text"
                    value={redeemInput}
                    onChange={e => setRedeemInput(e.target.value.toUpperCase())}
                    onKeyDown={e => e.key === 'Enter' && handleRedeem()}
                    placeholder="Enter code"
                    className="flex-1 px-4 py-2.5 bg-white/5 border border-white/10 rounded-xl font-mono text-sm tracking-wider placeholder:text-stone-500 placeholder:tracking-normal placeholder:font-sans focus:outline-none focus:ring-2 focus:ring-primary-500/50 focus:border-primary-500/50 transition-all"
                    disabled={redeemStatus === 'loading'}
                  />
                  <button
                    onClick={handleRedeem}
                    disabled={redeemStatus === 'loading' || !redeemInput.trim()}
                    className="btn-primary px-5 py-2.5 text-sm font-medium rounded-xl disabled:opacity-50 disabled:cursor-not-allowed whitespace-nowrap">
                    {redeemStatus === 'loading' ? '...' : 'Redeem'}
                  </button>
                </div>
                {redeemStatus === 'success' && (
                  <p className="text-sage-500 text-xs mt-2">Invite code redeemed successfully!</p>
                )}
                {redeemStatus === 'error' && redeemError && (
                  <p className="text-coral-500 text-xs mt-2">{redeemError}</p>
                )}
              </div>
            )}

            {/* Your Invite Codes */}
            <div className="bg-white rounded-2xl shadow-soft border border-stone-200 p-6 animate-fade-up">
              <div className="mb-4">
                <h2 className="text-lg font-bold mb-1">Your Invite Codes</h2>
                <p className="text-xs opacity-70">
                  Share these codes with friends. Each code can be used once.
                </p>
              </div>

              {loadError && <p className="text-coral-500 text-xs text-center py-2">{loadError}</p>}

              {isLoading ? (
                <div className="space-y-3">
                  {Array.from({ length: 5 }).map((_, i) => (
                    <div key={i} className="h-12 bg-white/5 rounded-xl animate-pulse" />
                  ))}
                </div>
              ) : codes.length > 0 ? (
                <div className="space-y-2">
                  {codes.map(invite => (
                    <CodeRow key={invite._id} invite={invite} />
                  ))}
                </div>
              ) : (
                <p className="text-sm text-stone-500 text-center py-6">
                  No invite codes available yet.
                </p>
              )}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
};

export default Invites;
