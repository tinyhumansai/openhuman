/**
 * Re-export shim over the split mock backend at `scripts/mock-api/`.
 *
 * Kept here so existing import paths (`scripts/mock-api-server.mjs`,
 * `app/test/e2e/mock-server.ts`, `scripts/test-rust-with-mock.sh`) keep
 * working without churn. New code should import from `./mock-api/index.mjs`.
 */
export {
  DEFAULT_PORT,
  clearRequestLog,
  getMockBehavior,
  getMockServerPort,
  getRequestLog,
  resetMockBehavior,
  setMockBehavior,
  setMockBehaviors,
  startMockServer,
  stopMockServer,
} from "./mock-api/index.mjs";
