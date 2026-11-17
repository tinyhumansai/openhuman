/**
 * What clicking Tiny (the mascot in the idle composer) does: start a live
 * voice session with the agent on the mascot stage beside the chat, or — where
 * no mascot stage is mounted — open the full-bleed Human page.
 */
export function openTinyVoice(
  chatMascot: { expandWithVoice: () => void } | null | undefined,
  navigate: (to: string) => void
): void {
  if (chatMascot) {
    chatMascot.expandWithVoice();
  } else {
    navigate('/human');
  }
}
