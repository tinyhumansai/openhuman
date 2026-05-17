// @ts-nocheck
/**
 * E2E mock server wrapper.
 *
 * Re-exports the shared mock backend used by app unit tests, app E2E,
 * and Rust tests (via scripts/mock-api-server.mjs + scripts/test-rust-with-mock.sh).
 */
export {
  clearRequestLog,
  emitMockAgentAudioStream,
  getMockBehavior,
  getRequestLog,
  resetMockBehavior,
  setMockBehavior,
  setMockBehaviors,
  startMockServer,
  stopMockServer,
} from '../../../scripts/mock-api-core.mjs';
