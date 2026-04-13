import ReferralRewardsSection from '../referral/ReferralRewardsSection';
import RewardsCouponSection from './RewardsCouponSection';

export default function RewardsReferralsTab() {
  return (
    <>
      <div className="rounded-2xl border border-stone-200 bg-white p-6 shadow-soft">
        <div className="space-y-2">
          <div className="inline-flex items-center gap-2 rounded-full border border-primary-200 bg-primary-50 px-3 py-1 text-xs font-medium text-primary-700">
            Referral Program
          </div>
          <h1 className="text-3xl font-semibold text-stone-900">Invite people into OpenHuman</h1>
          <p className="max-w-xl text-sm text-stone-600">
            Share your referral link, track your progress, and keep any active coupon rewards in
            one place.
          </p>
        </div>
      </div>

      <ReferralRewardsSection />
      <RewardsCouponSection />
    </>
  );
}
