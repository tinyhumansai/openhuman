/**
 * Shared data types for the isometric agent world.
 *
 * The world is *state-driven*: agents are not puppeteered frame-by-frame, they
 * are reconciled toward an authoritative {@link AgentState} that an external
 * controller (an AI backend, a test harness, or the on-screen debug panel)
 * pushes in. The renderer's job is to interpolate smoothly toward that state.
 */

/** Tile codes used inside a room layout matrix. */
export enum TileCode {
  Void = 0,
  Floor = 1,
  Wall = 2,
  Dais = 3,
  /** A short interior divider (cubicle wall) — blocks, but waist-high. */
  Partition = 4,
  /** Asphalt road surface (walkable, for streets and cars). */
  Road = 5,
  /** Light paved sidewalk / plaza tile. */
  Pavement = 6,
}

export type Facing = 'left' | 'right';

export type AgentAction = 'idle' | 'walking' | 'sitting' | 'inspecting';

/** One step of a resolved path: a tile plus the elevation level on that tile. */
export interface WalkNode {
  x: number;
  y: number;
  level: number;
}

export interface TileCoord {
  x: number;
  y: number;
}

export interface ChatMessage {
  text: string;
  /** Milliseconds the bubble stays before auto-fading. */
  durationMs?: number;
}

/**
 * Authoritative description of where an agent should be and what it is doing.
 * Everything except `x`/`y` is optional so a controller can send sparse updates.
 */
export interface AgentState {
  /** Target tile column. */
  x: number;
  /** Target tile row. */
  y: number;
  action?: AgentAction;
  facing?: Facing;
  /** Movement speed in tiles per second. */
  speed?: number;
  /** Display name shown on the nameplate. */
  label?: string;
  /** Body colour, used to give agents cheap visual diversity. */
  tint?: number;
  /** Optional one-shot chat line to speak on this update. */
  say?: ChatMessage;
}

/** A tile an agent can step onto to trigger a furniture interaction. */
export interface InteractionPoint {
  /** Tile offset from the furniture anchor where the agent stands. */
  tileOffsetX: number;
  tileOffsetY: number;
  action: 'sit' | 'inspect';
  facing?: Facing;
  /** Extra pixels to lower the agent so it rests *on* the seat. */
  seatDropY?: number;
}

/** Placement of one furniture piece inside a room. */
export interface FurnitureConfig {
  /** Builder key registered in the furniture kit. */
  kind: string;
  /** Anchor tile (top corner of the footprint). */
  tileX: number;
  tileY: number;
  /** Footprint size in tiles. Defaults to 1 x 1. */
  footprintWidth?: number;
  footprintHeight?: number;
  /** Elevation level the piece sits on. */
  level?: number;
  /** Whether the footprint blocks walking. Defaults to true. */
  solid?: boolean;
  /** Multiply tint applied to the shared base texture. */
  tint?: number;
  /** Stations agents can occupy. */
  interactionPoints?: Array<InteractionPoint>;
}

/** Five-shade palette that themes every room. */
export interface RoomPalette {
  background: number;
  floorTop: number;
  floorSide: number;
  wall: number;
  dais: number;
  accent: number;
}

/** Complete description of a room: its grid, palette, and furniture. */
export interface RoomDefinition {
  key: string;
  name: string;
  description: string;
  /** Layout matrix addressed as `matrix[row][column]` = `matrix[y][x]`. */
  matrix: Array<Array<TileCode>>;
  palette: RoomPalette;
  furniture: Array<FurnitureConfig>;
  /** Where newly spawned agents enter. */
  spawnTile: TileCoord;
  /** Extra vertical clearance above the floor for tall props (buildings). */
  topMargin?: number;
}
