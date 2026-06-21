import { useCallback, useEffect, useRef, useState } from 'react';

import { socketService } from '../../services/socketService';

export interface TinyplaceStreamMessage {
  stream_id: string;
  kind: string;
  message: unknown;
}

export interface TinyplaceStreamStatus {
  stream_id: string;
  status: string;
}

/**
 * Subscribe to tinyplace WebSocket stream events via the core's Socket.IO
 * bridge. The hook listens for `tinyplace:stream_message` and
 * `tinyplace:stream_status` events, optionally filtered by `streamId`.
 *
 * Returns:
 * - `messages` — received stream messages (capped at 100).
 * - `status` — latest lifecycle status: `"idle"` | `"connecting"` | `"connected"` | `"disconnected"` | `"failed"`.
 * - `clearMessages` — reset the messages array.
 */
export function useTinyplaceStream(streamId?: string) {
  const [messages, setMessages] = useState<TinyplaceStreamMessage[]>([]);
  const [status, setStatus] = useState<string>('idle');
  const messagesRef = useRef(messages);
  messagesRef.current = messages;

  const handleMessage = useCallback(
    (data: unknown) => {
      const msg = data as TinyplaceStreamMessage | null;
      if (!msg || typeof msg !== 'object') return;
      if (streamId !== undefined && msg.stream_id !== streamId) return;
      setMessages(prev => [...prev.slice(-99), msg]);
    },
    [streamId]
  );

  const handleStatus = useCallback(
    (data: unknown) => {
      const s = data as TinyplaceStreamStatus | null;
      if (!s || typeof s !== 'object') return;
      if (streamId !== undefined && s.stream_id !== streamId) return;
      setStatus(s.status);
    },
    [streamId]
  );

  useEffect(() => {
    socketService.on('tinyplace:stream_message', handleMessage);
    socketService.on('tinyplace:stream_status', handleStatus);
    return () => {
      socketService.off('tinyplace:stream_message', handleMessage);
      socketService.off('tinyplace:stream_status', handleStatus);
    };
  }, [handleMessage, handleStatus]);

  const clearMessages = useCallback(() => setMessages([]), []);

  return { messages, status, clearMessages };
}
