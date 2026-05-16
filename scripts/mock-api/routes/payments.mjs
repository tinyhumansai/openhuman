import { json } from "../http.mjs";
import { behavior } from "../state.mjs";

export function handlePayments(ctx) {
  const { method, url, parsedBody, res } = ctx;
  const mockBehavior = behavior();

  if (method === "GET" && /^\/payments\/credits\/balance\/?(\?.*)?$/.test(url)) {
    json(res, 200, {
      success: true,
      data: { balanceUsd: 10, topUpBalanceUsd: 0, topUpBaselineUsd: 0 },
    });
    return true;
  }

  if (
    method === "GET" &&
    (/^\/payments\/plan\/?(\?.*)?$/.test(url) ||
      /^\/payments\/stripe\/currentPlan\/?(\?.*)?$/.test(url))
  ) {
    const plan = mockBehavior.plan || "FREE";
    const isActive = mockBehavior.planActive === "true";
    const periodEnd = new Date(Date.now() + 30 * 86400000).toISOString();
    json(res, 200, {
      success: true,
      data: {
        plan,
        hasActiveSubscription: isActive,
        planExpiry: isActive ? periodEnd : null,
        subscription: isActive
          ? { id: "sub_mock_1", status: "active", currentPeriodEnd: periodEnd }
          : null,
      },
    });
    return true;
  }

  if (
    method === "POST" &&
    (/^\/payments\/stripe\/checkout\/?$/.test(url) ||
      /^\/payments\/stripe\/purchasePlan\/?$/.test(url))
  ) {
    if (mockBehavior.purchaseError === "true") {
      json(res, 500, { success: false, error: "Payment service unavailable" });
      return true;
    }
    json(res, 200, {
      success: true,
      data: {
        sessionId: "cs_mock_" + Date.now(),
        checkoutUrl: null,
      },
    });
    return true;
  }

  if (method === "POST" && /^\/payments\/stripe\/portal\/?$/.test(url)) {
    json(res, 200, {
      success: true,
      data: { portalUrl: "https://billing.stripe.com/mock-portal" },
    });
    return true;
  }

  if (method === "POST" && /^\/payments\/coinbase\/charge\/?$/.test(url)) {
    if (mockBehavior.coinbaseError === "true") {
      json(res, 500, { success: false, error: "Coinbase service unavailable" });
      return true;
    }
    json(res, 200, {
      success: true,
      data: {
        gatewayTransactionId: "charge_mock_" + Date.now(),
        hostedUrl: "https://commerce.coinbase.com/mock-charge",
        status: "NEW",
        expiresAt: new Date(Date.now() + 3600000).toISOString(),
      },
    });
    return true;
  }

  if (method === "POST" && /^\/payments\/purchase\/?$/.test(url)) {
    const plan = parsedBody?.plan || mockBehavior.plan || "BASIC";
    json(res, 200, {
      success: true,
      data: {
        sessionId: "cs_mock_" + Date.now(),
        url: "https://checkout.stripe.com/mock-purchase",
        plan,
      },
    });
    return true;
  }

  if (
    method === "GET" &&
    /^\/payments\/credits\/auto-recharge\/?(\?.*)?$/.test(url)
  ) {
    json(res, 200, {
      success: true,
      data: {
        enabled: false,
        thresholdUsd: 5,
        rechargeAmountUsd: 10,
        weeklyLimitUsd: 50,
        spentThisWeekUsd: 0,
        weekStartDate: new Date().toISOString(),
        inFlight: false,
        hasSavedPaymentMethod: false,
        lastTriggeredAt: null,
        lastRechargeAt: null,
      },
    });
    return true;
  }

  if (
    method === "PATCH" &&
    /^\/payments\/credits\/auto-recharge\/?$/.test(url)
  ) {
    // Gap fill: update auto-recharge config. Echo back the patched values.
    json(res, 200, {
      success: true,
      data: {
        enabled: parsedBody?.enabled ?? false,
        thresholdUsd: parsedBody?.thresholdUsd ?? 5,
        rechargeAmountUsd: parsedBody?.rechargeAmountUsd ?? 10,
        weeklyLimitUsd: parsedBody?.weeklyLimitUsd ?? 50,
        spentThisWeekUsd: 0,
        weekStartDate: new Date().toISOString(),
        inFlight: false,
        hasSavedPaymentMethod: false,
        lastTriggeredAt: null,
        lastRechargeAt: null,
      },
    });
    return true;
  }

  if (method === "GET" && /^\/payments\/cards\/?(\?.*)?$/.test(url)) {
    json(res, 200, { success: true, data: { cards: [], defaultCardId: null } });
    return true;
  }

  if (
    method === "GET" &&
    /^\/payments\/credits\/auto-recharge\/cards\/?(\?.*)?$/.test(url)
  ) {
    json(res, 200, { success: true, data: { cards: [], defaultCardId: null } });
    return true;
  }

  // ── Gap fills ─────────────────────────────────────────────────────

  if (
    method === "POST" &&
    /^\/payments\/credits\/auto-recharge\/cards\/setup-intent\/?$/.test(url)
  ) {
    json(res, 200, {
      success: true,
      data: {
        clientSecret: "seti_mock_" + Date.now() + "_secret_mock",
        setupIntentId: "seti_mock_" + Date.now(),
      },
    });
    return true;
  }

  if (
    method === "DELETE" &&
    /^\/payments\/credits\/auto-recharge\/cards\/[^/]+\/?$/.test(url)
  ) {
    json(res, 200, { success: true, data: { deleted: true } });
    return true;
  }

  if (
    method === "GET" &&
    /^\/payments\/credits\/transactions\/?(\?.*)?$/.test(url)
  ) {
    json(res, 200, {
      success: true,
      data: {
        transactions: [],
        nextCursor: null,
      },
    });
    return true;
  }

  if (method === "POST" && /^\/payments\/credits\/top-up\/?$/.test(url)) {
    // Don't collapse an explicit 0 into the default — `Number(0) || 10` is
    // a classic falsy-coalesce bug. Use Number.isFinite so any non-numeric
    // body still falls back to 10.
    const rawAmount = parsedBody?.amountUsd;
    const parsedAmount = rawAmount == null ? 10 : Number(rawAmount);
    const amount =
      Number.isFinite(parsedAmount) && parsedAmount >= 0 ? parsedAmount : 10;
    json(res, 200, {
      success: true,
      data: {
        sessionId: "cs_mock_topup_" + Date.now(),
        checkoutUrl: null,
        amountUsd: amount,
      },
    });
    return true;
  }

  if (
    method === "GET" &&
    /^\/payments\/coinbase\/charge\/[^/]+\/?(\?.*)?$/.test(url)
  ) {
    const status = mockBehavior.cryptoStatus || "NEW";
    json(res, 200, {
      success: true,
      data: {
        status,
        payment: {
          status,
          amountPaid:
            status === "UNDERPAID"
              ? "150.00"
              : status === "OVERPAID"
                ? "350.00"
                : "250.00",
          amountExpected: "250.00",
          currency: "USDC",
          underpaidAmount: mockBehavior.cryptoUnderpaidAmount || "0",
          overpaidAmount: mockBehavior.cryptoOverpaidAmount || "0",
        },
        expiresAt: new Date(Date.now() + 3600000).toISOString(),
      },
    });
    return true;
  }

  if (method === "GET" && /^\/billing\/current-plan\/?(\?.*)?$/.test(url)) {
    const plan = mockBehavior.plan || "FREE";
    const isActive = mockBehavior.planActive === "true";
    const expiry = mockBehavior.planExpiry || null;
    json(res, 200, {
      success: true,
      data: {
        plan,
        hasActiveSubscription: isActive,
        planExpiry: expiry,
        subscription: isActive
          ? {
              id: "sub_mock_123",
              status: "active",
              currentPeriodEnd:
                expiry || new Date(Date.now() + 30 * 86400000).toISOString(),
            }
          : null,
      },
    });
    return true;
  }

  return false;
}
