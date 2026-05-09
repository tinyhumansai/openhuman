import * as fs from 'node:fs';
import * as path from 'node:path';
import { describe, expect, test } from 'vitest';

import { CORE_RPC_METHODS, LEGACY_METHOD_ALIASES, normalizeRpcMethod } from '../rpcMethods';

describe('rpcMethods catalog', () => {
  describe('normalizeRpcMethod', () => {
    test('resolves all legacy aliases to their canonical core method', () => {
      for (const [legacyMethod, coreMethod] of Object.entries(LEGACY_METHOD_ALIASES)) {
        expect(normalizeRpcMethod(legacyMethod)).toBe(coreMethod);
      }
    });

    test('transforms auth methods by replacing dots with underscores', () => {
      expect(normalizeRpcMethod('openhuman.auth.login')).toBe('openhuman.auth_login');
      expect(normalizeRpcMethod('openhuman.auth.get.state')).toBe('openhuman.auth_get_state');
      expect(normalizeRpcMethod('openhuman.auth.a.b.c')).toBe('openhuman.auth_a_b_c');
    });

    test('transforms accessibility prefix to screen_intelligence prefix', () => {
      expect(normalizeRpcMethod('openhuman.accessibility_status')).toBe(
        'openhuman.screen_intelligence_status'
      );
      expect(normalizeRpcMethod('openhuman.accessibility_enable')).toBe(
        'openhuman.screen_intelligence_enable'
      );
    });

    test('returns unmapped or unrecognized methods unchanged', () => {
      expect(normalizeRpcMethod('openhuman.threads_list')).toBe('openhuman.threads_list');
      expect(normalizeRpcMethod('openhuman.unknown_method')).toBe('openhuman.unknown_method');
      expect(normalizeRpcMethod('')).toBe('');
      expect(normalizeRpcMethod('random_string')).toBe('random_string');
    });

    test('trims whitespace and converts to lower case', () => {
      expect(normalizeRpcMethod('  OpenHuman.Auth.Login  ')).toBe('openhuman.auth_login');
      expect(normalizeRpcMethod('  OPENHUMAN.GET_CONFIG ')).toBe(CORE_RPC_METHODS.configGet);
      expect(normalizeRpcMethod('OpenHuman.Accessibility_Status  ')).toBe(
        'openhuman.screen_intelligence_status'
      );
      expect(normalizeRpcMethod('   some_RANDOM_method  ')).toBe('some_random_method');
    });
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
    const schemaSources = [
      fs.readFileSync(
        path.resolve(__dirname, '../../../../src/openhuman/config/schemas.rs'),
        'utf8'
      ),
      fs.readFileSync(
        path.resolve(__dirname, '../../../../src/openhuman/screen_intelligence/schemas.rs'),
        'utf8'
      ),
    ].join('\n');

    for (const method of Object.values(CORE_RPC_METHODS)) {
      const methodRoot = method.slice('openhuman.'.length);
      const namespace = methodRoot.startsWith('screen_intelligence_')
        ? 'screen_intelligence'
        : 'config';
      const fnName = methodRoot.slice(`${namespace}_`.length);
      expect(schemaSources).toContain(`namespace: "${namespace}"`);
      expect(schemaSources).toContain(`function: "${fnName}"`);
    }
  });
});
