import { useCallback, useEffect, useRef, useState } from 'react';

import { useUser } from '../../hooks/useUser';
import { useT } from '../../lib/i18n/I18nContext';
import { useCoreState } from '../../providers/CoreStateProvider';
import { referralApi } from '../../services/api/referralApi';
import type { ReferralRelationshipStatus, ReferralStats } from '../../types/referral';
import { LATEST_APP_DOWNLOAD_URL } from '../../utils/config';

function formatUsd(n: number): string {
  return new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD' }).format(n);
}

function statusBadgeClass(status: ReferralRelationshipStatus): string {
  switch (status) {
    case 'converted':
      return 'bg-sage-100 dark:bg-sage-500/20 text-sage-800 dark:text-sage-200';
    case 'expired':
      return 'bg-stone-100 dark:bg-neutral-800 text-stone-600 dark:text-neutral-300';
    default:
      return 'bg-amber-50 dark:bg-amber-500/10 text-amber-800 dark:text-amber-200';
  }
}

const ReferralRewardsSection = () => {
  const { t } = useT();
  const { user, refetch } = useUser();
  const { snapshot } = useCoreState();
  const token = snapshot.sessionToken;

  const [stats, setStats] = useState<ReferralStats | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const [applyCode, setApplyCode] = useState('');
  const [applyLoading, setApplyLoading] = useState(false);
  const [applyError, setApplyError] = useState<string | null>(null);
  const [applySuccess, setApplySuccess] = useState(false);
  const [copyHint, setCopyHint] = useState<string | null>(null);

  const latestRequestIdRef = useRef(0);

  const loadStats = useCallback(async () => {
    if (!token) {
      latestRequestIdRef.current += 1;
      setLoading(false);
      return;
    }

    latestRequestIdRef.current += 1;
    const requestId = latestRequestIdRef.current;

    setLoading(true);
    setLoadError(null);
    try {
      const s = await referralApi.getStats();
      if (requestId !== latestRequestIdRef.current) return;
      setStats(s);
      console.debug('[referral-ui] stats', {
        codeLen: s.referralCode.length,
        referrals: s.referrals.length,
      });
    } catch (err) {
      if (requestId !== latestRequestIdRef.current) return;
      const msg =
        err && typeof err === 'object' && 'error' in err
          ? String((err as { error: string }).error)
          : 'Could not load referral stats';
      setLoadError(msg);
      console.debug('[referral-ui] stats error', msg);
    } finally {
      if (requestId === latestRequestIdRef.current) {
        setLoading(false);
      }
    }
  }, [token]);

  useEffect(() => {
    void loadStats();
  }, [loadStats]);

  const referralCodeToCopy = stats ? stats.referralCode.trim() : '';

  const handleCopy = async () => {
    if (!referralCodeToCopy) return;
    try {
      await navigator.clipboard.writeText(referralCodeToCopy);
      setCopyHint(t('common.copied'));
      setTimeout(() => setCopyHint(null), 2000);
    } catch {
      setCopyHint(t('rewards.referralSection.copyFailed'));
      setTimeout(() => setCopyHint(null), 2500);
    }
  };

  const handleShare = async () => {
    if (!referralCodeToCopy) return;
    const shareText = [
      'Join me on OpenHuman.',
      `Referral code: ${referralCodeToCopy}`,
      `Download OpenHuman: ${LATEST_APP_DOWNLOAD_URL}`,
    ].join('\n');

    try {
      if (navigator.share) {
        await navigator.share({ title: 'OpenHuman', text: shareText });
      } else {
        await navigator.clipboard.writeText(shareText);
        setCopyHint(t('common.copied'));
        setTimeout(() => setCopyHint(null), 2000);
      }
    } catch (e) {
      if ((e as Error)?.name !== 'AbortError') {
        try {
          await navigator.clipboard.writeText(shareText);
          setCopyHint(t('common.copied'));
          setTimeout(() => setCopyHint(null), 2000);
        } catch {
          setCopyHint(t('rewards.referralSection.copyFailed'));
          setTimeout(() => setCopyHint(null), 2500);
        }
      }
    }
  };

  const handleApply = async () => {
    const trimmed = applyCode.trim();
    if (!trimmed) return;
    const normalizedValue = trimmed.toUpperCase();
    setApplyLoading(true);
    setApplyError(null);
    try {
      await referralApi.claimReferral(normalizedValue);
      setApplySuccess(true);
      setApplyCode('');
      await refetch();
      await loadStats();
      console.debug('[referral-ui] apply completed');
    } catch (err) {
      const msg =
        err && typeof err === 'object' && 'error' in err
          ? String((err as { error: string }).error)
          : 'Could not apply referral code';
      setApplyError(msg);
    } finally {
      setApplyLoading(false);
    }
  };

  const hasAppliedFromProfile = !!user?.referral?.invitedBy || !!user?.referral?.invitedByCode;
  const hasAppliedFromStats =
    !!stats?.appliedReferralCode && stats.appliedReferralCode.trim() !== '';
  const showApplyForm =
    stats &&
    stats.canApplyReferral !== false &&
    !hasAppliedFromStats &&
    !hasAppliedFromProfile &&
    !applySuccess;

  if (!token) {
    return null;
  }

  return (
    <div className="space-y-4">
      <div className="bg-white dark:bg-neutral-900 rounded-2xl shadow-soft border border-stone-200 dark:border-neutral-800 p-6 space-y-6">
        <div className="flex flex-col gap-3 md:flex-row md:items-start md:justify-between">
          <div className="space-y-2">
            <h2 className="text-2xl font-semibold text-stone-900 dark:text-neutral-100">
              {t('rewards.referralSection.title')}
            </h2>
            <p className="text-sm text-stone-600 dark:text-neutral-300 max-w-xl">
              {t('rewards.referralSection.subtitle')}
            </p>
          </div>
        </div>

        {loading && !stats ? (
          <p className="text-sm text-stone-500 dark:text-neutral-400">
            {t('rewards.referralSection.loading')}
          </p>
        ) : null}
        {loadError ? (
          <div className="rounded-xl border border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 px-3 py-2 text-sm text-coral-800 dark:text-coral-200">
            {loadError}
            <button
              type="button"
              onClick={() => void loadStats()}
              className="ml-2 underline font-medium">
              {t('rewards.referralSection.retry')}
            </button>
          </div>
        ) : null}

        {stats ? (
          <>
            <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
              <div className="rounded-xl border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 p-4">
                <div className="text-xs font-medium uppercase tracking-wide text-stone-400 dark:text-neutral-500">
                  {t('rewards.referralSection.yourCode')}
                </div>
                <div className="mt-2 font-mono text-lg font-semibold text-stone-900 dark:text-neutral-100 break-all">
                  {stats.referralCode || '—'}
                </div>
              </div>
              <div className="rounded-xl border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 p-4">
                <div className="text-xs font-medium uppercase tracking-wide text-stone-400 dark:text-neutral-500">
                  {t('rewards.referralSection.totalEarned')}
                </div>
                <div className="mt-2 text-2xl font-semibold text-stone-900 dark:text-neutral-100">
                  {formatUsd(stats.totals.totalRewardUsd)}
                </div>
              </div>
              <div className="rounded-xl border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 p-4">
                <div className="text-xs font-medium uppercase tracking-wide text-stone-400 dark:text-neutral-500">
                  {t('rewards.referralSection.pendingReferrals')}
                </div>
                <div className="mt-2 text-2xl font-semibold text-stone-900 dark:text-neutral-100">
                  {stats.totals.pendingCount}
                </div>
              </div>
              <div className="rounded-xl border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 p-4">
                <div className="text-xs font-medium uppercase tracking-wide text-stone-400 dark:text-neutral-500">
                  {t('rewards.referralSection.completed')}
                </div>
                <div className="mt-2 text-2xl font-semibold text-stone-900 dark:text-neutral-100">
                  {stats.totals.convertedCount}
                </div>
              </div>
            </div>

            <div className="flex flex-col gap-2 sm:flex-row sm:flex-wrap">
              <button
                type="button"
                onClick={() => void handleCopy()}
                disabled={!referralCodeToCopy}
                className="inline-flex items-center justify-center gap-2 rounded-xl bg-stone-900 px-4 py-3 text-sm font-medium text-white transition-colors hover:bg-stone-800 disabled:opacity-50">
                {t('rewards.referralSection.copyCode')}
              </button>
              <button
                type="button"
                onClick={() => void handleShare()}
                disabled={!referralCodeToCopy}
                className="inline-flex items-center justify-center rounded-xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-4 py-3 text-sm font-medium text-stone-700 dark:text-neutral-200 transition-colors hover:bg-stone-50 dark:hover:bg-neutral-800/60 dark:bg-neutral-800/60 dark:hover:bg-neutral-800/60 disabled:opacity-50">
                {t('rewards.referralSection.share')}
              </button>
              {copyHint ? (
                <span className="self-center text-sm text-sage-600 dark:text-sage-300">
                  {copyHint}
                </span>
              ) : null}
            </div>
          </>
        ) : null}
      </div>

      {stats && stats.canApplyReferral !== false && showApplyForm ? (
        <div className="rounded-xl shadow-soft border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 p-4 space-y-3">
          <h2 className="text-2xl font-semibold text-stone-900 dark:text-neutral-100">
            {t('rewards.referralSection.haveCode')}
          </h2>
          <p className="text-xs text-stone-600 dark:text-neutral-300">
            {t('rewards.referralSection.haveCodeDesc')}
          </p>
          <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
            <input
              type="text"
              value={applyCode}
              onChange={e => setApplyCode(e.target.value)}
              onKeyDown={e => e.key === 'Enter' && void handleApply()}
              placeholder={t('rewards.referralSection.placeholder')}
              disabled={applyLoading}
              className="flex-1 px-4 py-2.5 rounded-xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 font-mono text-stone-900 dark:text-neutral-100 placeholder:text-stone-400 dark:text-neutral-500 dark:placeholder:text-neutral-500 focus:outline-none focus:ring-2 focus:ring-primary-500/40"
            />
            <button
              type="button"
              onClick={() => void handleApply()}
              disabled={applyLoading || !applyCode.trim()}
              className="rounded-xl bg-primary-600 px-4 py-2.5 text-sm font-medium text-white hover:bg-primary-700 disabled:opacity-50">
              {applyLoading
                ? t('rewards.referralSection.applying')
                : t('rewards.referralSection.apply')}
            </button>
          </div>
          {applyError ? (
            <p className="text-xs text-coral-600 dark:text-coral-300">{applyError}</p>
          ) : null}
        </div>
      ) : null}

      {stats && (hasAppliedFromStats || hasAppliedFromProfile || applySuccess) && !showApplyForm ? (
        <p className="text-sm text-sage-700 dark:text-sage-300 rounded-xl border border-sage-200 dark:border-sage-500/30 bg-sage-50 dark:bg-sage-500/10 px-3 py-2">
          {t('rewards.referralSection.linked')}
          {stats.appliedReferralCode
            ? ' ' +
              t('rewards.referralSection.linkedCode').replace('{code}', stats.appliedReferralCode)
            : ''}
          .
        </p>
      ) : null}

      {stats ? (
        <div className="bg-white dark:bg-neutral-900 rounded-2xl shadow-soft border border-stone-200 dark:border-neutral-800 p-6">
          <div>
            <h3 className="text-sm font-semibold text-stone-900 dark:text-neutral-100 mb-2">
              {t('rewards.referralSection.activity')}
            </h3>
            {stats.referrals.length === 0 ? (
              <p className="text-sm text-stone-500 dark:text-neutral-400 rounded-xl border border-dashed border-stone-200 dark:border-neutral-800 px-4 py-6 text-center">
                {t('rewards.referralSection.noReferrals')}
              </p>
            ) : (
              <div className="overflow-x-auto rounded-xl border border-stone-200 dark:border-neutral-800">
                <table className="min-w-full text-sm text-left">
                  <thead className="bg-stone-50 dark:bg-neutral-800/60 text-xs uppercase tracking-wide text-stone-500 dark:text-neutral-400">
                    <tr>
                      <th className="px-3 py-2 font-medium">
                        {t('rewards.referralSection.colReferredUser')}
                      </th>
                      <th className="px-3 py-2 font-medium">
                        {t('rewards.referralSection.colStatus')}
                      </th>
                      <th className="px-3 py-2 font-medium">
                        {t('rewards.referralSection.colReward')}
                      </th>
                      <th className="px-3 py-2 font-medium">
                        {t('rewards.referralSection.colUpdated')}
                      </th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-stone-100 dark:divide-neutral-800">
                    {stats.referrals.map((row, idx) => (
                      <tr
                        key={row.id ?? row.referredUserId ?? idx}
                        className="bg-white dark:bg-neutral-900">
                        <td className="px-3 py-2 font-mono text-stone-800 dark:text-neutral-100">
                          {row.referredUserMasked || row.referredDisplayName || '—'}
                        </td>
                        <td className="px-3 py-2">
                          <span
                            className={`inline-flex rounded-full px-2.5 py-0.5 text-xs font-medium ${statusBadgeClass(row.status)}`}>
                            {row.status === 'converted'
                              ? t('rewards.referralSection.statusCompleted')
                              : row.status === 'expired'
                                ? t('rewards.referralSection.statusExpired')
                                : t('rewards.referralSection.statusJoined')}
                          </span>
                        </td>
                        <td className="px-3 py-2 text-stone-700 dark:text-neutral-200">
                          {row.rewardUsd != null && row.rewardUsd > 0
                            ? formatUsd(row.rewardUsd)
                            : '—'}
                        </td>
                        <td className="px-3 py-2 text-stone-500 dark:text-neutral-400 text-xs">
                          {row.status === 'converted' && row.convertedAt
                            ? new Date(row.convertedAt).toLocaleString()
                            : row.createdAt
                              ? new Date(row.createdAt).toLocaleString()
                              : '—'}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </div>
        </div>
      ) : null}
    </div>
  );
};

export default ReferralRewardsSection;
