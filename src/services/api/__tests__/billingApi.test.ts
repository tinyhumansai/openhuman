import { describe, it, expect, vi, beforeEach } from "vitest";
import { billingApi } from "../billingApi";

// Mock the apiClient module
const mockGet = vi.fn();
const mockPost = vi.fn();

vi.mock("../../apiClient", () => ({
  apiClient: {
    get: (...args: unknown[]) => mockGet(...args),
    post: (...args: unknown[]) => mockPost(...args),
  },
}));

describe("billingApi", () => {
  beforeEach(() => {
    mockGet.mockReset();
    mockPost.mockReset();
  });

  describe("getCurrentPlan", () => {
    it("should call GET /payments/stripe/currentPlan", async () => {
      const planData = {
        plan: "BASIC",
        hasActiveSubscription: true,
        planExpiry: "2026-12-31T00:00:00.000Z",
        subscription: {
          id: "sub_123",
          status: "active",
          currentPeriodEnd: "2026-12-31T00:00:00.000Z",
        },
      };
      mockGet.mockResolvedValue({ success: true, data: planData });

      const result = await billingApi.getCurrentPlan();

      expect(mockGet).toHaveBeenCalledWith("/payments/stripe/currentPlan");
      expect(result).toEqual(planData);
    });

    it("should return FREE plan data for free users", async () => {
      const planData = {
        plan: "FREE",
        hasActiveSubscription: false,
        planExpiry: null,
        subscription: null,
      };
      mockGet.mockResolvedValue({ success: true, data: planData });

      const result = await billingApi.getCurrentPlan();

      expect(result.plan).toBe("FREE");
      expect(result.hasActiveSubscription).toBe(false);
      expect(result.subscription).toBeNull();
    });

    it("should propagate errors from apiClient", async () => {
      mockGet.mockRejectedValue({ success: false, error: "Unauthorized" });

      await expect(billingApi.getCurrentPlan()).rejects.toEqual({
        success: false,
        error: "Unauthorized",
      });
    });
  });

  describe("purchasePlan", () => {
    it("should call POST /payments/stripe/purchasePlan with plan ID", async () => {
      const checkoutData = {
        checkoutUrl: "https://checkout.stripe.com/c/pay/cs_test_123",
        sessionId: "cs_test_123",
      };
      mockPost.mockResolvedValue({ success: true, data: checkoutData });

      const result = await billingApi.purchasePlan("BASIC_MONTHLY");

      expect(mockPost).toHaveBeenCalledWith(
        "/payments/stripe/purchasePlan",
        { plan: "BASIC_MONTHLY" },
      );
      expect(result).toEqual(checkoutData);
    });

    it("should pass yearly plan IDs correctly", async () => {
      mockPost.mockResolvedValue({
        success: true,
        data: { checkoutUrl: "https://stripe.com/...", sessionId: "cs_456" },
      });

      await billingApi.purchasePlan("PRO_YEARLY");

      expect(mockPost).toHaveBeenCalledWith(
        "/payments/stripe/purchasePlan",
        { plan: "PRO_YEARLY" },
      );
    });

    it("should return null checkoutUrl when session creation has no URL", async () => {
      mockPost.mockResolvedValue({
        success: true,
        data: { checkoutUrl: null, sessionId: "cs_789" },
      });

      const result = await billingApi.purchasePlan("BASIC_MONTHLY");

      expect(result.checkoutUrl).toBeNull();
      expect(result.sessionId).toBe("cs_789");
    });

    it("should propagate errors from apiClient", async () => {
      mockPost.mockRejectedValue({
        success: false,
        error: "Invalid plan",
      });

      await expect(billingApi.purchasePlan("BASIC_MONTHLY")).rejects.toEqual({
        success: false,
        error: "Invalid plan",
      });
    });
  });

  describe("createPortalSession", () => {
    it("should call POST /payments/stripe/portal with no body", async () => {
      const portalData = {
        portalUrl: "https://billing.stripe.com/p/session/test_123",
      };
      mockPost.mockResolvedValue({ success: true, data: portalData });

      const result = await billingApi.createPortalSession();

      expect(mockPost).toHaveBeenCalledWith("/payments/stripe/portal");
      expect(result).toEqual(portalData);
    });

    it("should return the portal URL string", async () => {
      mockPost.mockResolvedValue({
        success: true,
        data: { portalUrl: "https://billing.stripe.com/session/abc" },
      });

      const result = await billingApi.createPortalSession();

      expect(result.portalUrl).toBe(
        "https://billing.stripe.com/session/abc",
      );
    });

    it("should propagate errors from apiClient", async () => {
      mockPost.mockRejectedValue({
        success: false,
        error: "Unable to resolve Stripe customer",
      });

      await expect(billingApi.createPortalSession()).rejects.toEqual({
        success: false,
        error: "Unable to resolve Stripe customer",
      });
    });
  });

  describe("createCoinbaseCharge", () => {
    it("should call POST /payments/coinbase/charge with plan and interval", async () => {
      const chargeData = {
        gatewayTransactionId: "charge_abc",
        hostedUrl: "https://commerce.coinbase.com/charges/abc",
        status: "created",
        expiresAt: "2026-01-31T12:15:00.000Z",
      };
      mockPost.mockResolvedValue({ success: true, data: chargeData });

      const result = await billingApi.createCoinbaseCharge("BASIC", "annual");

      expect(mockPost).toHaveBeenCalledWith("/payments/coinbase/charge", {
        plan: "BASIC",
        interval: "annual",
      });
      expect(result).toEqual(chargeData);
    });

    it("should default interval to annual", async () => {
      mockPost.mockResolvedValue({
        success: true,
        data: {
          gatewayTransactionId: "charge_xyz",
          hostedUrl: "https://commerce.coinbase.com/charges/xyz",
          status: "created",
          expiresAt: "2026-01-31T12:15:00.000Z",
        },
      });

      await billingApi.createCoinbaseCharge("PRO");

      expect(mockPost).toHaveBeenCalledWith("/payments/coinbase/charge", {
        plan: "PRO",
        interval: "annual",
      });
    });

    it("should return hosted URL for payment redirect", async () => {
      const expectedUrl = "https://commerce.coinbase.com/charges/test";
      mockPost.mockResolvedValue({
        success: true,
        data: {
          gatewayTransactionId: "charge_t",
          hostedUrl: expectedUrl,
          status: "created",
          expiresAt: "2026-01-31T12:15:00.000Z",
        },
      });

      const result = await billingApi.createCoinbaseCharge("BASIC", "annual");

      expect(result.hostedUrl).toBe(expectedUrl);
    });

    it("should propagate errors from apiClient", async () => {
      mockPost.mockRejectedValue({
        success: false,
        error: "Crypto payments are only available for annual plans",
      });

      await expect(
        billingApi.createCoinbaseCharge("BASIC", "annual"),
      ).rejects.toEqual({
        success: false,
        error: "Crypto payments are only available for annual plans",
      });
    });
  });
});
