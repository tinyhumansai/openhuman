import { describe, expect, it } from 'vitest';

import {
  endpointHost,
  isAzureFoundryEndpoint,
  isAzureV1BaseUrl,
  looksLikeAzureBaseModelId,
} from '../azureDeployment';

describe('endpointHost', () => {
  it('strips scheme, path, port and userinfo', () => {
    expect(endpointHost('https://my-res.openai.azure.com/openai/v1')).toBe(
      'my-res.openai.azure.com'
    );
    expect(endpointHost('MY-RES.OpenAI.Azure.com/openai/v1')).toBe('my-res.openai.azure.com');
    expect(endpointHost('https://user:pass@my-res.openai.azure.com:443/openai/v1')).toBe(
      'my-res.openai.azure.com'
    );
    expect(endpointHost('http://[::1]:8080/v1')).toBe('::1');
  });

  it('returns an empty string when there is no host', () => {
    expect(endpointHost('')).toBe('');
    expect(endpointHost('   ')).toBe('');
    expect(endpointHost(null)).toBe('');
    expect(endpointHost(undefined)).toBe('');
  });
});

describe('isAzureFoundryEndpoint', () => {
  it('recognises per-resource Azure endpoints', () => {
    for (const endpoint of [
      'https://my-res.openai.azure.com/openai/v1',
      'https://contoso.services.ai.azure.com/models',
      'https://my-res.cognitiveservices.azure.com/openai/v1',
      // Case and trailing path must not matter.
      'HTTPS://My-Res.OPENAI.AZURE.COM/openai/v1/',
      // Sovereign clouds: Azure Government and Azure operated by 21Vianet.
      // Separate DNS parents, so the commercial `.com` entries do not cover
      // them and a tenant there would otherwise hit the very "model not
      // found" path this module prevents.
      'https://my-res.openai.azure.us/openai/v1',
      'https://my-res.openai.azure.cn/openai/v1',
    ]) {
      expect(isAzureFoundryEndpoint(endpoint)).toBe(true);
    }
  });

  it('does not match non-Azure providers', () => {
    for (const endpoint of [
      'https://api.openai.com/v1',
      'https://api.groq.com/openai/v1',
      'http://localhost:11434/v1',
      'https://litellm.mycorp.dev/v1',
      // Foundry *serverless* endpoints. They speak the Azure AI Model
      // Inference API and key `model` on the model name, not a deployment
      // name, so relabelling their field would mislead rather than help.
      'https://team.inference.ai.azure.com/v1',
      'https://team.models.ai.azure.com/v1',
      '',
      null,
      undefined,
    ]) {
      expect(isAzureFoundryEndpoint(endpoint)).toBe(false);
    }
  });

  it('matches only on a dot boundary, not on a bare substring', () => {
    // A host that merely *contains* the domain must not match, or a
    // lookalike like `openai.azure.com.evil.test` would be trusted.
    expect(isAzureFoundryEndpoint('https://openai.azure.com.evil.test/v1')).toBe(false);
    // `myopenai.azure.com` is a different host under azure.com, not a
    // subdomain of `openai.azure.com`, so it must not match either.
    expect(isAzureFoundryEndpoint('https://myopenai.azure.com/v1')).toBe(false);
    // The genuine per-resource shape does match.
    expect(isAzureFoundryEndpoint('https://myopenai.openai.azure.com/v1')).toBe(true);
  });
});

describe('looksLikeAzureBaseModelId', () => {
  const catalog = ['gpt-5.6-terra-2026-07-09', 'gpt-4o'];

  it('flags a value taken verbatim from the provider catalog', () => {
    expect(looksLikeAzureBaseModelId('gpt-5.6-terra-2026-07-09', catalog)).toBe(true);
    expect(looksLikeAzureBaseModelId('  gpt-4o  ', catalog)).toBe(true);
  });

  it('does not flag a deployment name that is absent from the catalog', () => {
    // The whole point of the fix: a deployment name is normally NOT in the
    // catalog, and that state must read as correct rather than suspicious.
    expect(looksLikeAzureBaseModelId('gpt-5.6-terra', catalog)).toBe(false);
  });

  it('does not flag empty values or an empty catalog', () => {
    expect(looksLikeAzureBaseModelId('', catalog)).toBe(false);
    expect(looksLikeAzureBaseModelId('   ', catalog)).toBe(false);
    expect(looksLikeAzureBaseModelId(null, catalog)).toBe(false);
    expect(looksLikeAzureBaseModelId('gpt-4o', [])).toBe(false);
  });
});

describe('isAzureV1BaseUrl', () => {
  it('accepts only the OpenAI-compatible v1 base', () => {
    expect(isAzureV1BaseUrl('https://my-res.openai.azure.com/openai/v1')).toBe(true);
    expect(isAzureV1BaseUrl('https://my-res.openai.azure.com/openai/v1/')).toBe(true);
    expect(isAzureV1BaseUrl('HTTPS://My-Res.OPENAI.AZURE.COM/openai/v1')).toBe(true);
    expect(isAzureV1BaseUrl('https://contoso.services.ai.azure.com/openai/v1')).toBe(true);
  });

  it('rejects the classic api-version surface and bare resource URLs', () => {
    // These serve no `{base}/models` listing and want an `api-key` header, so
    // both the add-provider probe and inference fail on them.
    expect(isAzureV1BaseUrl('https://my-res.openai.azure.com/openai')).toBe(false);
    expect(isAzureV1BaseUrl('https://my-res.openai.azure.com')).toBe(false);
    expect(isAzureV1BaseUrl('https://my-res.openai.azure.com/openai/deployments/my-dep')).toBe(
      false
    );
  });

  it('is false for a non-Azure endpoint', () => {
    expect(isAzureV1BaseUrl('https://api.openai.com/v1')).toBe(false);
    expect(isAzureV1BaseUrl('')).toBe(false);
    expect(isAzureV1BaseUrl(null)).toBe(false);
  });
});
