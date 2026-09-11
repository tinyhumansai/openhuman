export {
  ChatMascotProvider,
  type ChatMascotContextValue,
  useChatMascot,
  useChatMascotOptional,
  useChatMascotSendBinding,
} from './ChatMascotContext';
export { default as ChatMascotDock } from './ChatMascotDock';
export { default as ChatMascotOverlay } from './ChatMascotOverlay';
export { default as ChatMascotStage } from './ChatMascotStage';
export {
  DOCK_PX as MASCOT_DOCK_PX,
  prefersReducedMotion,
  STAGE_RENDER_PX as MASCOT_STAGE_RENDER_PX,
  TRANSITION_MS as MASCOT_TRANSITION_MS,
} from './geometry';
export { type ChatMascotSendBinding, ChatMascotSendStore } from './sendBinding';
