/**
 * A scripted, offline assistant-ui turn.
 *
 * The dev-only upstream demo renders assistant-ui against a stand-in model so
 * it can exercise every transcript part without touching OpenHuman data. The
 * product chat uses the external Redux runtime and never imports this folder.
 *
 * Nothing in here reaches the core, the backend, or any provider.
 */
export { mockChatModelAdapter } from './mockChatModel';
export { buildSeedMessages, MOCK_SCRIPT, SEED_PROMPT } from './mockScript';
export type { MockStep, MockSubagentResult, MockSubagentStep } from './mockScript';
export { MockToolFallback, MockToolGroup, SubagentCall } from './SubagentCall';
