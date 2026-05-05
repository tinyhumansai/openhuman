export type ComposerSendBlockReason =
  | 'empty_input'
  | 'missing_thread'
  | 'composer_blocked'
  | 'usage_limit_reached'
  | 'socket_disconnected';

export type SlashCommandDecision =
  | { kind: 'new_or_clear'; blockedByWelcomeLock: boolean }
  | { kind: 'not_handled' };

export interface ComposerSendDecisionArgs {
  rawText: string;
  selectedThreadId: string | null;
  composerInteractionBlocked: boolean;
  isAtLimit: boolean;
  socketStatus: string;
}

export interface ComposerSendDecision {
  shouldSend: boolean;
  trimmedText: string;
  blockReason?: ComposerSendBlockReason;
}

export const handleComposerSlashCommand = (
  command: string,
  welcomeLocked: boolean
): SlashCommandDecision => {
  const cmd = command.toLowerCase();
  if (cmd === '/new' || cmd === '/clear') {
    return { kind: 'new_or_clear', blockedByWelcomeLock: welcomeLocked };
  }
  return { kind: 'not_handled' };
};

export const evaluateComposerSend = (args: ComposerSendDecisionArgs): ComposerSendDecision => {
  const trimmedText = args.rawText.trim();

  if (!trimmedText) {
    return { shouldSend: false, trimmedText, blockReason: 'empty_input' };
  }

  if (!args.selectedThreadId) {
    return { shouldSend: false, trimmedText, blockReason: 'missing_thread' };
  }

  if (args.composerInteractionBlocked) {
    return { shouldSend: false, trimmedText, blockReason: 'composer_blocked' };
  }

  if (args.isAtLimit) {
    return { shouldSend: false, trimmedText, blockReason: 'usage_limit_reached' };
  }

  if (args.socketStatus !== 'connected') {
    return { shouldSend: false, trimmedText, blockReason: 'socket_disconnected' };
  }

  return { shouldSend: true, trimmedText };
};
