/** Structured chat send / delivery errors (issue #219) — stable `code` for analytics and tests. */

export type ChatSendErrorCode =
  | 'socket_disconnected'
  | 'local_model_failed'
  | 'cloud_send_failed'
  | 'voice_transcription'
  | 'microphone_unavailable'
  | 'microphone_recording'
  | 'microphone_access'
  | 'voice_playback'
  | 'safety_timeout';

export interface ChatSendError {
  code: ChatSendErrorCode;
  message: string;
}

export function chatSendError(
  code: ChatSendErrorCode,
  message: string,
): ChatSendError {
  return { code, message };
}
