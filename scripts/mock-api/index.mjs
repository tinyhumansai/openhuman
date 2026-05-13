/**
 * Public surface of the e2e mock backend. Re-exports the lifecycle + state
 * helpers consumed by:
 *   - `scripts/mock-api-server.mjs` (CLI runner)
 *   - `scripts/test-rust-with-mock.sh` (Rust integration tests)
 *   - `app/test/e2e/mock-server.ts` (WDIO specs + Vitest unit setup)
 *
 * The legacy entrypoint at `scripts/mock-api-core.mjs` is a re-export shim
 * over this module so existing import paths keep working.
 */
export { getMockServerPort, startMockServer, stopMockServer } from "./server.mjs";
export {
  DEFAULT_PORT,
  clearRequestLog,
  getMockBehavior,
  getRequestLog,
  resetMockBehavior,
  setMockBehavior,
  setMockBehaviors,
} from "./state.mjs";
