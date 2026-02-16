/**
 * MSW request handlers for API mocking in tests.
 *
 * These provide deterministic API responses for testing components
 * that depend on backend data.
 */
import { http, HttpResponse } from 'msw';

const BACKEND_URL = 'http://localhost:5005';

export const handlers = [
  // GET /telegram/me - Current user profile
  http.get(`${BACKEND_URL}/telegram/me`, () => {
    return HttpResponse.json({
      success: true,
      data: {
        _id: 'user-123',
        telegramId: 12345678,
        hasAccess: true,
        magicWord: 'alpha',
        firstName: 'Test',
        lastName: 'User',
        username: 'testuser',
        role: 'user' as const,
        activeTeamId: 'team-1',
        referral: {},
        subscription: { hasActiveSubscription: false, plan: 'FREE' as const },
        settings: {
          dailySummariesEnabled: false,
          dailySummaryChatIds: [],
          autoCompleteEnabled: false,
          autoCompleteVisibility: 'always' as const,
          autoCompleteWhitelistChatIds: [],
          autoCompleteBlacklistChatIds: [],
        },
        usage: {
          cycleBudgetUsd: 10,
          spentThisCycleUsd: 0,
          spentTodayUsd: 0,
          cycleStartDate: new Date().toISOString(),
        },
        autoDeleteTelegramMessagesAfterDays: 30,
        autoDeleteThreadsAfterDays: 30,
      },
    });
  }),

  // GET /billing/current-plan
  http.get(`${BACKEND_URL}/billing/current-plan`, () => {
    return HttpResponse.json({
      success: true,
      data: { plan: 'FREE', hasActiveSubscription: false, planExpiry: null, subscription: null },
    });
  }),

  // GET /teams
  http.get(`${BACKEND_URL}/teams`, () => {
    return HttpResponse.json({ success: true, data: [] });
  }),

  // POST /auth/desktop-exchange
  http.post(`${BACKEND_URL}/auth/desktop-exchange`, () => {
    return HttpResponse.json({
      sessionToken: 'mock-session-token',
      user: { id: 'user-123', firstName: 'Test', username: 'testuser' },
    });
  }),
];
