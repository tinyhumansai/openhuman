/** Public surface of the isometric agent world engine. */

export { GameWorld, RendererInitError } from './GameWorld';
export type { AgentSummary } from './GameWorld';
export { BaseRoom } from './BaseRoom';
export {
  ROOM_REGISTRY,
  PokerTableRoom,
  CourtHouseRoom,
  OfficeRoom,
  HomeRoom,
  OutsideWorldRoom,
} from './rooms';
export type { RoomEntry } from './rooms';
export { ChatBubble } from './ChatBubble';
export { Agent } from './Agent';
export { FurnitureSprite, FURNITURE_BLUEPRINTS } from './furniture';
export { TextureFactory } from './textures';
export type {
  AgentAction,
  AgentState,
  ChatMessage,
  Facing,
  FurnitureConfig,
  InteractionPoint,
  RoomDefinition,
  RoomPalette,
  WalkNode,
} from './types';
export { TileCode } from './types';
