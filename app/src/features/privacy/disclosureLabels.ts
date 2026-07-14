import type { EgressDataKind, EgressReason } from '../../services/chatService';
import type { PrivacyMode } from '../../store/privacySlice';

/**
 * Map the snake_case egress enums to friendly, human-readable i18n keys so the
 * disclosure surface never renders raw wire strings (#4437 / S3). Each mapper
 * tolerates unknown/future values by falling back to a generic key.
 */

export function dataKindLabelKey(kind: EgressDataKind | string): string {
  switch (kind) {
    case 'prompt':
      return 'privacy.disclosure.kind.prompt';
    case 'tool_arguments':
      return 'privacy.disclosure.kind.toolArguments';
    case 'embedding_input':
      return 'privacy.disclosure.kind.embeddingInput';
    case 'file_content':
      return 'privacy.disclosure.kind.fileContent';
    case 'url':
      return 'privacy.disclosure.kind.url';
    case 'metadata':
      return 'privacy.disclosure.kind.metadata';
    default:
      return 'privacy.disclosure.kind.unknown';
  }
}

export function reasonLabelKey(reason: EgressReason | string): string {
  switch (reason) {
    case 'inference':
      return 'privacy.disclosure.reason.inference';
    case 'tool_call':
      return 'privacy.disclosure.reason.toolCall';
    case 'integration':
      return 'privacy.disclosure.reason.integration';
    case 'embedding':
      return 'privacy.disclosure.reason.embedding';
    case 'network_fetch':
      return 'privacy.disclosure.reason.networkFetch';
    default:
      return 'privacy.disclosure.reason.unknown';
  }
}

export function privacyModeLabelKey(mode: PrivacyMode): string {
  switch (mode) {
    case 'local_only':
      return 'privacy.mode.localOnly';
    case 'standard':
      return 'privacy.mode.standard';
    case 'sensitive':
      return 'privacy.mode.sensitive';
  }
}
