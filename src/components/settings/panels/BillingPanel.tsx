import { useState, useEffect, useRef, useCallback } from "react";
import { useAppSelector, useAppDispatch } from "../../../store/hooks";
import { fetchCurrentUser } from "../../../store/userSlice";
import { useSettingsNavigation } from "../hooks/useSettingsNavigation";
import SettingsHeader from "../components/SettingsHeader";
import { billingApi } from "../../../services/api/billingApi";
import { openUrl } from "../../../utils/openUrl";
import type { CurrentPlanData, PlanTier } from "../../../types/api";
import {
  PLANS,
  buildPlanId,
  displayPrice,
  annualSavings,
  isUpgrade as checkIsUpgrade,
} from "./billingHelpers";

// ── Component ───────────────────────────────────────────────────────────
const BillingPanel = () => {
  const { navigateBack } = useSettingsNavigation();
  const dispatch = useAppDispatch();
  const user = useAppSelector((state) => state.user.user);

  // Derived from Redux user state
  const currentTier: PlanTier = user?.subscription?.plan ?? "FREE";
  const hasActive = user?.subscription?.hasActiveSubscription ?? false;
  const planExpiry = user?.subscription?.planExpiry;
  const usage = user?.usage;

  // Local state
  const [billingInterval, setBillingInterval] = useState<"monthly" | "annual">(
    "monthly",
  );
  const [paymentMethod, setPaymentMethod] = useState<"card" | "crypto">(
    "card",
  );
  const [isLoading, setIsLoading] = useState(false);
  const [isPurchasing, setIsPurchasing] = useState(false);
  const [purchasingTier, setPurchasingTier] = useState<PlanTier | null>(null);
  const [currentPlanData, setCurrentPlanData] =
    useState<CurrentPlanData | null>(null);
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const pollStartRef = useRef<number>(0);

  // Fetch current plan on mount
  useEffect(() => {
    setIsLoading(true);
    billingApi
      .getCurrentPlan()
      .then(setCurrentPlanData)
      .catch(console.error)
      .finally(() => setIsLoading(false));
  }, []);

  // When crypto is selected, force annual
  useEffect(() => {
    if (paymentMethod === "crypto") {
      setBillingInterval("annual");
    }
  }, [paymentMethod]);

  // Cleanup poll on unmount
  useEffect(() => {
    return () => {
      if (pollRef.current) clearInterval(pollRef.current);
    };
  }, []);

  // ── Poll for plan change after checkout ─────────────────────────────
  const startPolling = useCallback(() => {
    if (pollRef.current) clearInterval(pollRef.current);
    pollStartRef.current = Date.now();

    pollRef.current = setInterval(async () => {
      // Stop after 2 minutes
      if (Date.now() - pollStartRef.current > 120_000) {
        if (pollRef.current) clearInterval(pollRef.current);
        setIsPurchasing(false);
        setPurchasingTier(null);
        return;
      }

      try {
        const plan = await billingApi.getCurrentPlan();
        if (
          plan.hasActiveSubscription &&
          plan.plan !== currentTier
        ) {
          setCurrentPlanData(plan);
          dispatch(fetchCurrentUser());
          setIsPurchasing(false);
          setPurchasingTier(null);
          if (pollRef.current) clearInterval(pollRef.current);
        }
      } catch {
        // Ignore polling errors
      }
    }, 5_000);
  }, [currentTier, dispatch]);

  // ── Purchase handlers ───────────────────────────────────────────────
  const handleUpgrade = async (tier: PlanTier) => {
    if (tier === "FREE" || tier === currentTier) return;
    setIsPurchasing(true);
    setPurchasingTier(tier);

    try {
      if (paymentMethod === "crypto") {
        const { hostedUrl } = await billingApi.createCoinbaseCharge(
          tier,
          "annual",
        );
        await openUrl(hostedUrl);
      } else {
        const planId = buildPlanId(tier, billingInterval);
        const { checkoutUrl } = await billingApi.purchasePlan(planId);
        if (checkoutUrl) await openUrl(checkoutUrl);
      }
      startPolling();
    } catch (err) {
      console.error("Purchase failed:", err);
      setIsPurchasing(false);
      setPurchasingTier(null);
    }
  };

  const handleManageSubscription = async () => {
    try {
      const { portalUrl } = await billingApi.createPortalSession();
      await openUrl(portalUrl);
    } catch (err) {
      console.error("Portal session failed:", err);
    }
  };

  // ── JSX ─────────────────────────────────────────────────────────────
  return (
    <div className="overflow-hidden h-full flex flex-col">
      <SettingsHeader
        title="Billing & Subscription"
        showBackButton={true}
        onBack={navigateBack}
      />

      <div className="flex-1 overflow-y-auto">
        <div className="p-4 space-y-5">
          {/* ── Current plan banner ──────────────────────────────── */}
          <div className="rounded-2xl bg-stone-800/60 border border-stone-700/50 p-4">
            <div className="flex items-center justify-between mb-3">
              <div className="flex items-center gap-2">
                <h3 className="text-sm font-semibold text-white">
                  {isLoading
                    ? "Loading..."
                    : (currentPlanData?.plan ?? currentTier)}{" "}
                  Plan
                </h3>
                <span
                  className={`px-2 py-0.5 text-xs font-medium rounded-full ${
                    hasActive
                      ? "bg-sage-500/20 text-sage-400 border border-sage-500/30"
                      : "bg-stone-600/30 text-stone-400 border border-stone-600/40"
                  }`}
                >
                  {hasActive ? "Active" : "Free"}
                </span>
              </div>

              {hasActive && (
                <button
                  onClick={handleManageSubscription}
                  className="text-xs text-primary-400 hover:text-primary-300 font-medium transition-colors"
                >
                  Manage Subscription
                </button>
              )}
            </div>

            {/* Renewal date */}
            {hasActive && planExpiry && (
              <p className="text-xs text-stone-400 mb-3">
                Renews{" "}
                {new Date(planExpiry).toLocaleDateString("en-US", {
                  month: "long",
                  day: "numeric",
                  year: "numeric",
                })}
              </p>
            )}

            {/* Token usage */}
            {usage && (
              <div>
                <div className="flex items-center justify-between text-xs text-stone-400 mb-1.5">
                  <span>Daily token usage</span>
                  <span>
                    {usage.remainingTokens.toLocaleString()} /{" "}
                    {usage.dailyTokenLimit.toLocaleString()} remaining
                  </span>
                </div>
                <div className="h-1.5 bg-stone-700/60 rounded-full overflow-hidden">
                  <div
                    className="h-full rounded-full transition-all duration-300 bg-primary-500"
                    style={{
                      width: `${Math.min(
                        100,
                        (usage.remainingTokens / usage.dailyTokenLimit) * 100,
                      )}%`,
                    }}
                  />
                </div>
              </div>
            )}
          </div>

          {/* ── Interval toggle ──────────────────────────────────── */}
          <div className="flex items-center justify-center gap-2">
            <button
              onClick={() => {
                if (paymentMethod !== "crypto") setBillingInterval("monthly");
              }}
              disabled={paymentMethod === "crypto"}
              className={`px-3 py-1.5 text-xs font-medium rounded-lg transition-colors ${
                billingInterval === "monthly"
                  ? "bg-primary-500/20 text-primary-400 border border-primary-500/30"
                  : "text-stone-400 hover:text-stone-300"
              } ${paymentMethod === "crypto" ? "opacity-40 cursor-not-allowed" : ""}`}
            >
              Monthly
            </button>
            <button
              onClick={() => setBillingInterval("annual")}
              className={`px-3 py-1.5 text-xs font-medium rounded-lg transition-colors ${
                billingInterval === "annual"
                  ? "bg-primary-500/20 text-primary-400 border border-primary-500/30"
                  : "text-stone-400 hover:text-stone-300"
              }`}
            >
              Annual
            </button>
          </div>

          {/* ── Plan tier cards ───────────────────────────────────── */}
          <div className="space-y-3">
            {PLANS.map((plan) => {
              const isCurrent = plan.tier === currentTier;
              const isUpgrade = checkIsUpgrade(plan.tier, currentTier);
              const savings = annualSavings(plan, billingInterval);
              const isThisPurchasing =
                isPurchasing && purchasingTier === plan.tier;

              return (
                <div
                  key={plan.tier}
                  className={`rounded-2xl border p-4 transition-all ${
                    isCurrent
                      ? "border-primary-500/40 bg-primary-500/5"
                      : "border-stone-700/50 bg-stone-800/40"
                  }`}
                >
                  <div className="flex items-start justify-between mb-3">
                    <div>
                      <div className="flex items-center gap-2">
                        <h4 className="text-sm font-semibold text-white">
                          {plan.name}
                        </h4>
                        {isCurrent && (
                          <span className="px-1.5 py-0.5 text-[10px] font-medium rounded-full bg-primary-500/20 text-primary-400 border border-primary-500/30">
                            Current
                          </span>
                        )}
                        {savings && (
                          <span className="px-1.5 py-0.5 text-[10px] font-medium rounded-full bg-sage-500/20 text-sage-400 border border-sage-500/30">
                            Save {savings}%
                          </span>
                        )}
                      </div>
                      <div className="mt-1 flex items-baseline gap-1">
                        <span className="text-xl font-bold text-white">
                          {displayPrice(plan, billingInterval)}
                        </span>
                        {plan.tier !== "FREE" && (
                          <span className="text-xs text-stone-400">/mo</span>
                        )}
                        {plan.tier !== "FREE" &&
                          billingInterval === "annual" && (
                            <span className="text-xs text-stone-500 ml-1">
                              (billed ${plan.annualPrice}/yr)
                            </span>
                          )}
                      </div>
                    </div>

                    {/* Action button */}
                    {isUpgrade && (
                      <button
                        onClick={() => handleUpgrade(plan.tier)}
                        disabled={isPurchasing}
                        className={`px-3 py-1.5 text-xs font-medium rounded-lg transition-colors ${
                          isPurchasing
                            ? "bg-stone-700/40 text-stone-500 cursor-not-allowed"
                            : "bg-primary-500 hover:bg-primary-600 text-white"
                        }`}
                      >
                        {isThisPurchasing ? "Waiting..." : "Upgrade"}
                      </button>
                    )}
                  </div>

                  {/* Features */}
                  <ul className="space-y-1.5">
                    {plan.features.map((f) => (
                      <li
                        key={f.text}
                        className="flex items-center gap-2 text-xs text-stone-300"
                      >
                        <svg
                          className="w-3.5 h-3.5 text-sage-500 flex-shrink-0"
                          fill="none"
                          stroke="currentColor"
                          viewBox="0 0 24 24"
                        >
                          <path
                            strokeLinecap="round"
                            strokeLinejoin="round"
                            strokeWidth={2}
                            d="M5 13l4 4L19 7"
                          />
                        </svg>
                        {f.text}
                      </li>
                    ))}
                  </ul>
                </div>
              );
            })}
          </div>

          {/* ── Purchasing overlay message ────────────────────────── */}
          {isPurchasing && (
            <div className="rounded-xl bg-amber-500/10 border border-amber-500/20 p-3">
              <div className="flex items-center gap-2">
                <svg
                  className="w-4 h-4 text-amber-400 animate-spin"
                  fill="none"
                  viewBox="0 0 24 24"
                >
                  <circle
                    className="opacity-25"
                    cx="12"
                    cy="12"
                    r="10"
                    stroke="currentColor"
                    strokeWidth="4"
                  />
                  <path
                    className="opacity-75"
                    fill="currentColor"
                    d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
                  />
                </svg>
                <p className="text-xs text-amber-300">
                  Waiting for payment confirmation... Complete checkout in the
                  browser window that opened.
                </p>
              </div>
            </div>
          )}

          {/* ── Pay with crypto toggle ────────────────────────────── */}
          <div className="flex items-center justify-between rounded-xl bg-stone-800/40 border border-stone-700/40 p-3">
            <div>
              <p className="text-xs font-medium text-white">
                Pay with Crypto
              </p>
              <p className="text-[11px] text-stone-400 mt-0.5">
                Annual plans only via Coinbase Commerce
              </p>
            </div>
            <button
              onClick={() =>
                setPaymentMethod((m) => (m === "card" ? "crypto" : "card"))
              }
              className={`relative w-10 h-5 rounded-full transition-colors ${
                paymentMethod === "crypto"
                  ? "bg-primary-500"
                  : "bg-stone-600"
              }`}
              role="switch"
              aria-checked={paymentMethod === "crypto"}
            >
              <span
                className={`absolute top-0.5 left-0.5 w-4 h-4 rounded-full bg-white transition-transform ${
                  paymentMethod === "crypto"
                    ? "translate-x-5"
                    : "translate-x-0"
                }`}
              />
            </button>
          </div>

          {/* ── Info notice ───────────────────────────────────────── */}
          <div className="p-3 bg-blue-500/10 border border-blue-500/20 rounded-xl">
            <div className="flex items-start gap-2">
              <svg
                className="w-4 h-4 text-blue-400 flex-shrink-0 mt-0.5"
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24"
              >
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"
                />
              </svg>
              <p className="text-[11px] text-blue-200">
                Payments processed securely through Stripe. Crypto payments
                available for annual plans via Coinbase.
              </p>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
};

export default BillingPanel;
