import { describe, expect, it } from 'vitest';

import {
  authStyleForBuiltinCloudProvider,
  BUILTIN_CLOUD_PROVIDER_SLUGS,
  BUILTIN_CLOUD_PROVIDERS,
  defaultEndpointForBuiltinCloudProvider,
} from '../builtinCloudProviders';

describe('builtinCloudProviders', () => {
  it('keeps built-in provider slugs unique', () => {
    expect(new Set(BUILTIN_CLOUD_PROVIDER_SLUGS).size).toBe(BUILTIN_CLOUD_PROVIDER_SLUGS.length);
  });

  it.each([
    ['groq', 'https://api.groq.com/openai/v1', 'bearer'],
    ['deepseek', 'https://api.deepseek.com/v1', 'bearer'],
    ['minimax', 'https://api.minimax.io/v1', 'bearer'],
    ['sumopod', 'https://ai.sumopod.com/v1', 'bearer'],
    ['modelscope', 'https://api-inference.modelscope.cn/v1', 'bearer'],
  ] as const)('maps %s to its endpoint and auth style', (slug, endpoint, authStyle) => {
    expect(defaultEndpointForBuiltinCloudProvider(slug)).toBe(endpoint);
    expect(authStyleForBuiltinCloudProvider(slug)).toBe(authStyle);
  });

  it('contains the full phase one provider set', () => {
    expect(BUILTIN_CLOUD_PROVIDERS.map(provider => provider.slug)).toEqual(
      expect.arrayContaining([
        'groq',
        'mistral',
        'deepseek',
        'together',
        'google',
        'cerebras',
        'xai',
        'huggingface',
        'nvidia',
        'zai',
        'minimax',
        'stepfun',
        'kilocode',
        'deepinfra',
        'novita',
        'venice',
        'vercel-ai-gateway',
        'sumopod',
        'modelscope',
      ])
    );
  });
});
