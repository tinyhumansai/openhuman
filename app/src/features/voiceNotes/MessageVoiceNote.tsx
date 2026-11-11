/**
 * MessageVoiceNote - the voice-note bubble body for a thread message.
 *
 * A message is a voice note when its persisted `extraMetadata.voiceNote` points
 * at stored audio. A user note replaces the text bubble (the text is the
 * transcript, shown in a disclosure); an assistant reply keeps its text and
 * gets a player under it, since the text already is the transcript.
 */
import { useAuiState } from '@assistant-ui/react';

import { readVoiceNoteMeta, type VoiceNoteMeta } from './voiceNoteApi';
import { VoiceNotePlayer } from './VoiceNotePlayer';

/** The message's voice-note reference, or `null` for an ordinary message. */
export function useMessageVoiceNote(): VoiceNoteMeta | null {
  const custom = useAuiState(s => s.message.metadata?.custom) as
    | { extraMetadata?: unknown }
    | undefined;
  return readVoiceNoteMeta(custom?.extraMetadata);
}

function useMessageText(): string {
  const parts = useAuiState(s => s.message.parts);
  return parts
    .map(part => (part.type === 'text' ? part.text : ''))
    .join('')
    .trim();
}

export function MessageVoiceNote({
  note,
  withTranscript,
}: {
  note: VoiceNoteMeta;
  withTranscript: boolean;
}) {
  const text = useMessageText();
  return (
    <VoiceNotePlayer
      noteId={note.noteId}
      durationMs={note.durationMs}
      waveform={note.waveform}
      visemes={note.visemes}
      mascotPlayback={!withTranscript}
      speechText={text}
      transcript={withTranscript ? text : undefined}
    />
  );
}

export default MessageVoiceNote;
