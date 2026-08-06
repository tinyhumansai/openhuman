/**
 * Realtime voice-agent session bootstrap (#5399).
 *
 * Mints a short-lived signed WebSocket URL for an ElevenLabs Agents
 * (Conversational AI) session via the core RPC `openhuman.voice_agent_signed_url`,
 * which proxies the backend `/voice-agent/get-signed-url`. The renderer never
 * sees the provider API key — only the short-lived signed URL — so the realtime
 * session client opens the connection with it directly.
 */
import { callCoreRpc } from '../coreRpcClient';

/** Camel-cased result the renderer consumes. */
export interface VoiceAgentSignedUrl {
  signedUrl: string;
  agentId: string;
  /**
   * Short-lived token binding this session to the signed-in user. Passed back as
   * the ElevenLabs `userId` so the backend relay verifies it instead of trusting
   * a raw id (#5399). Empty only against a backend that predates the binding.
   */
  userToken: string;
}

/** Snake-cased wire shape returned by the core RPC. */
interface VoiceAgentSignedUrlRpc {
  signed_url: string;
  agent_id: string;
  user_token?: string;
}

/**
 * Fetch a fresh signed URL for a realtime voice-agent session. Throws the core
 * RPC error (e.g. "no backend session token" when signed out, or a backend 400
 * when the agent is not configured) so callers can surface it.
 */
export async function fetchVoiceAgentSignedUrl(): Promise<VoiceAgentSignedUrl> {
  const result = await callCoreRpc<VoiceAgentSignedUrlRpc>({
    method: 'openhuman.voice_agent_signed_url',
  });
  return {
    signedUrl: result.signed_url,
    agentId: result.agent_id,
    userToken: result.user_token ?? '',
  };
}
