import { describe, expect, it } from 'vitest';

import {
  getRequiredFieldsForToolkit,
  TOOLKIT_REQUIRED_FIELDS,
  validateRequiredFieldValues,
} from './toolkitRequiredFields';

describe('toolkitRequiredFields registry', () => {
  it('exposes the Dynamics 365 org_name field with the .crm.dynamics.com suffix', () => {
    const fields = getRequiredFieldsForToolkit('dynamics365');
    expect(fields).toHaveLength(1);
    expect(fields[0].key).toBe('org_name');
    expect(fields[0].suffix).toBe('.crm.dynamics.com');
    expect(fields[0].placeholderKey).toBe('composio.connect.dynamicsOrgNamePlaceholder');
  });

  it('exposes the Jira subdomain field with the .atlassian.net suffix', () => {
    const fields = getRequiredFieldsForToolkit('jira');
    expect(fields).toHaveLength(1);
    expect(fields[0].key).toBe('subdomain');
    expect(fields[0].suffix).toBe('.atlassian.net');
  });

  it('exposes the WhatsApp waba_id field with no suffix', () => {
    const fields = getRequiredFieldsForToolkit('whatsapp');
    expect(fields).toHaveLength(1);
    expect(fields[0].key).toBe('waba_id');
    expect(fields[0].suffix).toBeUndefined();
  });

  it('returns an empty array for toolkits with no required fields (e.g. gmail)', () => {
    expect(getRequiredFieldsForToolkit('gmail')).toEqual([]);
    expect(getRequiredFieldsForToolkit('unknown-future-toolkit')).toEqual([]);
  });

  it('is exposed as a readonly frozen object', () => {
    expect(Object.isFrozen(TOOLKIT_REQUIRED_FIELDS)).toBe(true);
  });
});

describe('validateRequiredFieldValues', () => {
  const dynamicsFields = getRequiredFieldsForToolkit('dynamics365');

  it('returns an empty error map when all required fields are valid', () => {
    expect(validateRequiredFieldValues(dynamicsFields, { org_name: 'acme' })).toEqual({});
  });

  it('flags empty values with the requiredFieldEmpty i18n key', () => {
    expect(validateRequiredFieldValues(dynamicsFields, {})).toEqual({
      org_name: 'composio.connect.requiredFieldEmpty',
    });
    expect(validateRequiredFieldValues(dynamicsFields, { org_name: '   ' })).toEqual({
      org_name: 'composio.connect.requiredFieldEmpty',
    });
  });

  it('flags a full URL with the subdomain-invalid i18n key (custom validator)', () => {
    expect(
      validateRequiredFieldValues(dynamicsFields, { org_name: 'https://acme.crm.dynamics.com' })
    ).toEqual({ org_name: 'composio.connect.subdomainInvalid' });
  });

  it('flags leading/trailing hyphens for subdomain fields', () => {
    expect(validateRequiredFieldValues(dynamicsFields, { org_name: '-acme' })).toEqual({
      org_name: 'composio.connect.subdomainInvalid',
    });
    expect(validateRequiredFieldValues(dynamicsFields, { org_name: 'acme-' })).toEqual({
      org_name: 'composio.connect.subdomainInvalid',
    });
  });

  it('skips custom validation for fields without a validator (whatsapp waba_id)', () => {
    const whatsappFields = getRequiredFieldsForToolkit('whatsapp');
    // Non-empty waba_id is accepted with no format check (Composio validates server-side).
    expect(validateRequiredFieldValues(whatsappFields, { waba_id: '123abc' })).toEqual({});
    // Empty is still rejected with the generic required-field error.
    expect(validateRequiredFieldValues(whatsappFields, {})).toEqual({
      waba_id: 'composio.connect.requiredFieldEmpty',
    });
  });
});
