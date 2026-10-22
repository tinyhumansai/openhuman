import { describe, expect, it } from 'vitest';

import type { OciSettings } from '../../../../utils/tauriCommands/config';
import { toWireCloudProvider } from './aiPanelTypes';
import {
  DEFAULT_OCI_SETTINGS,
  isOciGatedPrincipalMode,
  isOciPrincipalMode,
  isOciProfileMode,
  normalizeOciSettings,
  OCI_AUTH_MODE_OPTIONS,
  OCI_TRANSPORT_OPTIONS,
  ociEndpointOverride,
  ociSettingsSummary,
  validateOciSettings,
} from './ociSettings';

const settings = (patch: Partial<OciSettings> = {}): OciSettings => ({
  ...DEFAULT_OCI_SETTINGS,
  region: 'us-chicago-1',
  project_id: 'ocid1.generativeaiproject.x',
  ...patch,
});

describe('validateOciSettings', () => {
  it('requires a region in api_key mode', () => {
    expect(validateOciSettings(settings({ region: '  ' }), 'key', false)).toBe(
      'settings.ai.oci.regionRequired'
    );
  });

  it('requires a key in api_key mode unless one is already stored', () => {
    expect(validateOciSettings(settings(), '  ', false)).toBe('settings.ai.apiKeyRequired');
    expect(validateOciSettings(settings(), '', true)).toBeNull();
    expect(validateOciSettings(settings(), 'sk', false)).toBeNull();
  });

  it('lets a profile or principal supply the region and needs no key', () => {
    for (const mode of OCI_AUTH_MODE_OPTIONS.map(o => o.value).filter(v => v !== 'api_key')) {
      expect(
        validateOciSettings(
          settings({ auth_mode: mode, region: '', compartment_id: 'ocid1.compartment.x' }),
          '',
          false
        )
      ).toBeNull();
    }
  });

  it('requires a compartment for signed auth even on openai', () => {
    for (const mode of OCI_AUTH_MODE_OPTIONS.map(o => o.value).filter(v => v !== 'api_key')) {
      expect(
        validateOciSettings(settings({ auth_mode: mode, transport: 'openai' }), '', false)
      ).toBe('settings.ai.oci.compartmentRequired');
    }
  });

  it('requires a compartment for every transport but openai with an API key', () => {
    for (const { value } of OCI_TRANSPORT_OPTIONS) {
      const result = validateOciSettings(settings({ transport: value }), 'key', false);
      expect(result).toBe(value === 'openai' ? null : 'settings.ai.oci.compartmentRequired');
    }
    expect(
      validateOciSettings(
        settings({ transport: 'native_cohere', compartment_id: 'ocid1.compartment.x' }),
        'key',
        false
      )
    ).toBeNull();
  });
});

describe('validateOciSettings project', () => {
  it('requires a project only on the openai transport', () => {
    expect(validateOciSettings(settings({ project_id: '  ' }), 'key', false)).toBe(
      'settings.ai.oci.projectRequired'
    );
    expect(
      validateOciSettings(
        settings({ transport: 'actions', project_id: '', compartment_id: 'ocid1.compartment.x' }),
        'key',
        false
      )
    ).toBeNull();
  });
});

describe('ociEndpointOverride', () => {
  it('blanks the preset display URL so the core routes by region', () => {
    expect(ociEndpointOverride(undefined)).toBe('');
    expect(ociEndpointOverride('  ')).toBe('');
    expect(
      ociEndpointOverride('https://inference.generativeai.us-chicago-1.oci.oraclecloud.com/')
    ).toBe('');
  });

  it('keeps an explicit override', () => {
    expect(ociEndpointOverride(' https://genai.private.example.oraclecloud.com ')).toBe(
      'https://genai.private.example.oraclecloud.com'
    );
  });
});

describe('normalizeOciSettings', () => {
  it('trims values and drops empty optional fields', () => {
    expect(
      normalizeOciSettings(
        settings({ region: ' eu-frankfurt-1 ', compartment_id: '  ', project_id: ' ' })
      )
    ).toEqual({ auth_mode: 'api_key', transport: 'openai', region: 'eu-frankfurt-1' });
  });

  it('drops profile fields that do not apply to the chosen mode', () => {
    const base = settings({ config_file: '~/.oci/config', profile: 'DEFAULT' });
    expect(normalizeOciSettings(base)).not.toHaveProperty('profile');
    expect(normalizeOciSettings(base)).not.toHaveProperty('config_file');
    expect(normalizeOciSettings({ ...base, auth_mode: 'session_token' })).toMatchObject({
      config_file: '~/.oci/config',
      profile: 'DEFAULT',
    });
  });

  it('drops the project id when the transport does not send it', () => {
    const base = settings({ project_id: 'ocid1.project.x', compartment_id: 'ocid1.compartment.x' });
    expect(normalizeOciSettings(base).project_id).toBe('ocid1.project.x');
    expect(normalizeOciSettings({ ...base, transport: 'native_generic' })).not.toHaveProperty(
      'project_id'
    );
  });
});

describe('mode classification and summary', () => {
  it('classifies every mode exactly once', () => {
    for (const { value } of OCI_AUTH_MODE_OPTIONS) {
      const kinds = [value === 'api_key', isOciProfileMode(value), isOciPrincipalMode(value)];
      expect(kinds.filter(Boolean)).toHaveLength(1);
    }
    expect(isOciGatedPrincipalMode('instance_principal')).toBe(true);
    expect(isOciGatedPrincipalMode('oke_workload_identity')).toBe(true);
    expect(isOciGatedPrincipalMode('resource_principal')).toBe(false);
  });

  it('summarises region and mode for the provider row', () => {
    const t = (key: string) => `t:${key}`;
    expect(ociSettingsSummary(settings({ auth_mode: 'session_token' }), t)).toBe(
      'us-chicago-1 · t:settings.ai.oci.mode.sessionToken'
    );
    expect(ociSettingsSummary(undefined, t)).toBe('');
  });
});

describe('toWireCloudProvider', () => {
  const base = {
    id: 'p_oci',
    slug: 'oci',
    label: 'OCI Generative AI',
    endpoint: 'https://inference.generativeai.us-chicago-1.oci.oraclecloud.com',
    authStyle: 'oci' as const,
    maskedKey: 'Not configured',
  };

  it('carries the oci block to the wire', () => {
    const oci = settings({ auth_mode: 'config_profile' });
    expect(toWireCloudProvider({ ...base, oci })).toEqual({
      id: 'p_oci',
      slug: 'oci',
      label: 'OCI Generative AI',
      endpoint: base.endpoint,
      ca_cert_pem: '',
      auth_style: 'oci',
      oci,
    });
  });

  it('omits the oci key entirely for other providers', () => {
    expect(
      toWireCloudProvider({ ...base, slug: 'openai', authStyle: 'bearer' })
    ).not.toHaveProperty('oci');
  });
});
