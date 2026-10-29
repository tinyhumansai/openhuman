import { describe, expect, it } from 'vitest';

import { providerEndpointIssue } from '../providerEndpointPolicy';

describe('providerEndpointIssue', () => {
  it('allows existing valid HTTPS provider endpoints and local HTTP development', () => {
    for (const endpoint of [
      'https://api.example.com/v1',
      'https://api.example.com:8443/models',
      'http://localhost:11434/v1',
      'http://127.0.0.1:9000/v1',
      'http://[::1]:8080/v1',
    ]) {
      expect(providerEndpointIssue(endpoint, true)).toBeNull();
    }
  });

  it('preserves intentionally unauthenticated HTTP endpoint settings', () => {
    expect(providerEndpointIssue('http://192.168.1.17:9000/v1', false)).toBeNull();
    expect(providerEndpointIssue('', false)).toBeNull();
  });

  it('refuses a keyed remote HTTP destination before a credential is stored', () => {
    expect(providerEndpointIssue('http://192.168.1.17:9000/v1', true)).toBe('insecure_bearer');
    expect(providerEndpointIssue('http://api.example.com/v1', true)).toBe('insecure_bearer');
    expect(providerEndpointIssue('http://localhost.evil.example/v1', true)).toBe('insecure_bearer');
  });

  it('rejects URL-embedded credentials, query parameters and fragments', () => {
    for (const endpoint of [
      'https://user:secret@api.example.com/v1',
      'https://api.example.com/v1?key=hidden',
      'https://api.example.com/v1#hidden',
    ]) {
      expect(providerEndpointIssue(endpoint, true)).toBe('invalid_url');
      expect(providerEndpointIssue(endpoint, false)).toBe('invalid_url');
    }
  });

  it('does not normalize away controls or permit non-HTTP schemes', () => {
    for (const endpoint of [
      'https://api.example.com/\nprivate',
      'https://api.example.com/\tprivate',
      'javascript:alert(1)',
      'file:///etc/passwd',
      'ftp://api.example.com/v1',
      'not a URL',
    ]) {
      expect(providerEndpointIssue(endpoint, true)).toBe('invalid_url');
    }
  });
});
