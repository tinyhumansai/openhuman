import { apiClient } from '../../../services/apiClient';

/**
 * One frame on the ElevenLabs viseme timeline. Backend emits the Oculus /
 * Microsoft 15-set: `sil, PP, FF, TH, DD, kk, CH, SS, nn, RR, aa, E, I, O, U`.
 */
export interface VisemeFrame {
  viseme: string;
  start_ms: number;
  end_ms: number;
}

export interface AlignmentFrame {
  char: string;
  start_ms: number;
  end_ms: number;
}

export interface TtsResponse {
  audio_base64: string;
  /** mime, e.g. "audio/mpeg". */
  audio_mime?: string;
  visemes: VisemeFrame[];
  alignment?: AlignmentFrame[];
}

export interface TtsOptions {
  voiceId?: string;
  modelId?: string;
  outputFormat?: string;
}

export async function synthesizeSpeech(text: string, opts: TtsOptions = {}): Promise<TtsResponse> {
  const body: Record<string, unknown> = { text, with_visemes: true };
  if (opts.voiceId) body.voice_id = opts.voiceId;
  if (opts.modelId) body.model_id = opts.modelId;
  if (opts.outputFormat) body.output_format = opts.outputFormat;
  return apiClient.post<TtsResponse>('/openai/v1/audio/speech', body);
}
