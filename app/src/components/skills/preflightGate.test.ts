import { describe, expect, it } from 'vitest';

import { isGithubGateFailure, parseSkillRunError } from './preflightGate';

describe('parseSkillRunError', () => {
  it('returns the raw body unchanged when no preflight prefix is present', () => {
    const out = parseSkillRunError('Run failed because foo');
    expect(out.gate).toBeNull();
    expect(out.tag).toBeNull();
    expect(out.body).toBe('Run failed because foo');
  });

  it('handles null / undefined / empty without throwing', () => {
    expect(parseSkillRunError(null).body).toBe('');
    expect(parseSkillRunError(undefined).body).toBe('');
    expect(parseSkillRunError('').body).toBe('');
  });

  it('parses a github identity_mismatch failure into gate + tag + body', () => {
    const raw =
      '[preflight:github:identity_mismatch] GitHub preflight failed: identity mismatch — Composio is `octo-alice` but git is `Alice`.';
    const out = parseSkillRunError(raw);
    expect(out.gate).toBe('github');
    expect(out.tag).toBe('identity_mismatch');
    expect(out.body).toContain('GitHub preflight failed');
    expect(out.body).not.toContain('[preflight');
  });

  it('parses every documented github gate tag', () => {
    const tags = [
      'composio_github_missing',
      'git_binary_missing',
      'git_user_name_missing',
      'git_user_email_missing',
      'identity_mismatch',
      'composio_identity_unresolved',
    ];
    for (const tag of tags) {
      const raw = `[preflight:github:${tag}] body for ${tag}`;
      const out = parseSkillRunError(raw);
      expect(out.gate).toBe('github');
      expect(out.tag).toBe(tag);
      expect(out.body).toBe(`body for ${tag}`);
    }
  });

  it('is idempotent — re-parsing a stripped body is a no-op', () => {
    const raw = '[preflight:github:git_user_name_missing] please set user.name';
    const once = parseSkillRunError(raw);
    const twice = parseSkillRunError(once.body);
    expect(twice.gate).toBeNull();
    expect(twice.tag).toBeNull();
    expect(twice.body).toBe(once.body);
  });

  it('tolerates lowercase / mixed case in the prefix gate name', () => {
    const out = parseSkillRunError('[preflight:GITHUB:Identity_Mismatch] body');
    expect(out.gate).toBe('github');
    expect(out.tag).toBe('identity_mismatch');
    expect(out.body).toBe('body');
  });

  it('does NOT parse a prefix-shaped string with a stray prefix-like text', () => {
    // Anchored at start of string only — a dump that contains the
    // prefix mid-text shouldn't be misinterpreted.
    const raw = 'orchestrator log: [preflight:github:tag] not the head of the string';
    const out = parseSkillRunError(raw);
    expect(out.gate).toBeNull();
    expect(out.body).toBe(raw);
  });
});

describe('isGithubGateFailure', () => {
  it('returns true for a github-gate parsed error', () => {
    const err = parseSkillRunError('[preflight:github:identity_mismatch] x');
    expect(isGithubGateFailure(err)).toBe(true);
  });

  it('returns false for a free-form error', () => {
    const err = parseSkillRunError('Something else failed');
    expect(isGithubGateFailure(err)).toBe(false);
  });

  it('returns false for a future non-github gate (forward-compat)', () => {
    const err = parseSkillRunError('[preflight:slack:scope_missing] body');
    expect(isGithubGateFailure(err)).toBe(false);
    expect(err.gate).toBe('slack');
  });
});
