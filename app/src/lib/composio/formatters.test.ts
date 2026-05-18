import { describe, expect, it } from 'vitest';

import { formatComposioToolError, formatTriggerLabel } from './formatters';

describe('formatComposioToolError', () => {
  it('strips the classified prefix and returns the body', () => {
    const raw =
      '[composio:error:insufficient_scope] `GMAIL_FETCH_EMAILS` was rejected because the connected gmail account is missing required permissions.';
    expect(formatComposioToolError(raw)).toContain('missing required permissions');
    expect(formatComposioToolError(raw)).not.toContain('[composio:error:');
  });

  it('passes through unclassified messages', () => {
    expect(formatComposioToolError('plain failure')).toBe('plain failure');
  });

  it('returns empty string for null/undefined/empty input', () => {
    expect(formatComposioToolError(null)).toBe('');
    expect(formatComposioToolError(undefined)).toBe('');
    expect(formatComposioToolError('')).toBe('');
  });

  it('falls back to validation copy when body is empty', () => {
    expect(formatComposioToolError('[composio:error:validation]')).toBe('Invalid tool arguments.');
  });

  it('falls back to insufficient_scope copy when body is empty', () => {
    expect(formatComposioToolError('[composio:error:insufficient_scope]')).toBe(
      'Reconnect this integration and grant the requested permissions.'
    );
  });

  it('falls back to rate_limited copy when body is empty', () => {
    expect(formatComposioToolError('[composio:error:rate_limited]')).toBe(
      'The upstream service is rate-limiting requests. Try again shortly.'
    );
  });

  it('falls back to gateway copy when body is empty', () => {
    expect(formatComposioToolError('[composio:error:gateway]')).toBe(
      'Temporary connection issue. Try again in a moment.'
    );
  });

  it('returns body for unknown class when present', () => {
    expect(formatComposioToolError('[composio:error:something_new] details here')).toBe(
      'details here'
    );
  });

  it('falls back to trimmed raw for unknown class with empty body', () => {
    expect(formatComposioToolError('  [composio:error:something_new]  ')).toBe(
      '[composio:error:something_new]'
    );
  });
});

describe('formatTriggerLabel', () => {
  it('formats GOOGLECALENDAR_GOOGLE_CALENDAR_EVENT_CREATED_TRIGGER correctly', () => {
    expect(formatTriggerLabel('GOOGLECALENDAR_GOOGLE_CALENDAR_EVENT_CREATED_TRIGGER')).toBe(
      'Google Calendar Event Created'
    );
  });

  it('formats GITHUB_ISSUE_OPENED correctly', () => {
    expect(formatTriggerLabel('GITHUB_ISSUE_OPENED')).toBe('GitHub Issue Opened');
  });

  it('formats SLACK_MESSAGE_RECEIVED_TRIGGER correctly', () => {
    expect(formatTriggerLabel('SLACK_MESSAGE_RECEIVED_TRIGGER')).toBe('Slack Message Received');
  });

  it('handles empty string and undefined', () => {
    expect(formatTriggerLabel('')).toBe('');
    expect(formatTriggerLabel(undefined as any)).toBe('');
    expect(formatTriggerLabel(null as any)).toBe('');
  });

  it('respects overrides', () => {
    const overrides = { GITHUB_ISSUE_OPENED: 'New Issue on GitHub' };
    expect(formatTriggerLabel('GITHUB_ISSUE_OPENED', { overrides })).toBe('New Issue on GitHub');
  });

  it('dedupes leading provider prefix when it matches next token', () => {
    expect(formatTriggerLabel('SLACK_SLACK_MESSAGE_RECEIVED')).toBe('Slack Message Received');
  });

  it('handles case-insensitivity for _TRIGGER suffix', () => {
    expect(formatTriggerLabel('GITHUB_PUSH_trigger')).toBe('GitHub Push');
  });

  it('handles tokens with multiple consecutive underscores correctly', () => {
    expect(formatTriggerLabel('LINEAR__ISSUE___CREATED')).toBe('Linear Issue Created');
  });

  it('honors explicit empty-string override (hasOwnProperty path)', () => {
    expect(formatTriggerLabel('NOOP', { overrides: { NOOP: '' } })).toBe('');
  });
});
