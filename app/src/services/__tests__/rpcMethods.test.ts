import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, test } from 'vitest';

import { CORE_RPC_METHODS, LEGACY_METHOD_ALIASES, normalizeRpcMethod } from '../rpcMethods';

describe('rpcMethods catalog', () => {
  test('normalizes legacy aliases and prefixes', () => {
    expect(normalizeRpcMethod('openhuman.get_config')).toBe(CORE_RPC_METHODS.configGet);
    expect(normalizeRpcMethod('openhuman.auth.get_state')).toBe('openhuman.auth_get_state');
    expect(normalizeRpcMethod('openhuman.accessibility_status')).toBe(
      CORE_RPC_METHODS.screenIntelligenceStatus
    );
    expect(normalizeRpcMethod('openhuman.threads_list')).toBe('openhuman.threads_list');
  });

  test('legacy aliases point at canonical method values', () => {
    expect(LEGACY_METHOD_ALIASES['openhuman.update_model_settings']).toBe(
      CORE_RPC_METHODS.configUpdateModelSettings
    );
    expect(LEGACY_METHOD_ALIASES['openhuman.workspace_onboarding_flag_set']).toBe(
      CORE_RPC_METHODS.configWorkspaceOnboardingFlagSet
    );
  });

  test('catalog canonical methods exist in core schema registry (drift guard)', () => {
    const coreAll = fs.readFileSync(path.resolve(__dirname, '../../../../src/core/all.rs'), 'utf8');
    for (const method of Object.values(CORE_RPC_METHODS)) {
      expect(coreAll).toContain(method);
    }
  });
});
