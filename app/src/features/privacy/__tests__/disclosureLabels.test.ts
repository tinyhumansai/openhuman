import { describe, expect, it } from 'vitest';

import type { EgressDataKind, EgressReason } from '../../../services/chatService';
import type { PrivacyMode } from '../../../store/privacySlice';
import { dataKindLabelKey, privacyModeLabelKey, reasonLabelKey } from '../disclosureLabels';

describe('disclosureLabels', () => {
  it('maps every data kind to a distinct friendly i18n key', () => {
    const kinds: EgressDataKind[] = [
      'prompt',
      'tool_arguments',
      'embedding_input',
      'file_content',
      'url',
      'metadata',
    ];
    const keys = kinds.map(dataKindLabelKey);
    expect(keys).toEqual([
      'privacy.disclosure.kind.prompt',
      'privacy.disclosure.kind.toolArguments',
      'privacy.disclosure.kind.embeddingInput',
      'privacy.disclosure.kind.fileContent',
      'privacy.disclosure.kind.url',
      'privacy.disclosure.kind.metadata',
    ]);
    // No collisions.
    expect(new Set(keys).size).toBe(keys.length);
  });

  it('falls back to the generic kind key for an unknown value', () => {
    expect(dataKindLabelKey('some_future_kind')).toBe('privacy.disclosure.kind.unknown');
  });

  it('maps every reason to a distinct friendly i18n key', () => {
    const reasons: EgressReason[] = [
      'inference',
      'tool_call',
      'integration',
      'embedding',
      'network_fetch',
    ];
    const keys = reasons.map(reasonLabelKey);
    expect(keys).toEqual([
      'privacy.disclosure.reason.inference',
      'privacy.disclosure.reason.toolCall',
      'privacy.disclosure.reason.integration',
      'privacy.disclosure.reason.embedding',
      'privacy.disclosure.reason.networkFetch',
    ]);
    expect(new Set(keys).size).toBe(keys.length);
  });

  it('falls back to the generic reason key for an unknown value', () => {
    expect(reasonLabelKey('some_future_reason')).toBe('privacy.disclosure.reason.unknown');
  });

  it('maps every privacy mode to its label key', () => {
    const modes: PrivacyMode[] = ['local_only', 'standard', 'sensitive'];
    expect(modes.map(privacyModeLabelKey)).toEqual([
      'privacy.mode.localOnly',
      'privacy.mode.standard',
      'privacy.mode.sensitive',
    ]);
  });

  it('falls back to the standard mode key for an unexpected mode value', () => {
    // Guards the always-visible pill against `t(undefined)` if the core ever
    // emits a mode string the client does not yet know (#4437 finding 5).
    expect(privacyModeLabelKey('some_future_mode')).toBe('privacy.mode.standard');
  });
});
