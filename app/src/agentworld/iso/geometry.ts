/**
 * Isometric geometry for the agent world.
 *
 * The grid uses a classic 2:1 isometric projection: a tile is twice as wide as
 * it is tall (64 x 32 by default). Tile `(gridX, gridY)` projects to the screen
 * with the diamond's *top* corner at {@link tileToScreen}; the diamond then
 * spans one tile-width to the east/west and one tile-height down to the south.
 *
 * Elevation (`level`) lifts a tile straight up the screen by a fixed number of
 * pixels per level, which is what gives the courthouse dais and raised furniture
 * their height. Depth sorting collapses the world to a single painter's-order
 * scalar derived from `gridX + gridY + level`, exactly the "global screen depth"
 * the renderer needs.
 */

export const TILE_WIDTH = 64;
export const TILE_HEIGHT = 32;
export const HALF_TILE_WIDTH = TILE_WIDTH / 2;
export const HALF_TILE_HEIGHT = TILE_HEIGHT / 2;

/** Screen pixels a single elevation level lifts a tile upward. */
export const ELEVATION_HEIGHT = 26;

/** Native (unscaled) resolution of the game viewport, in logical pixels. */
export const NATIVE_RESOLUTION = 600;

export interface ScreenPoint {
  x: number;
  y: number;
}

export interface TilePoint {
  x: number;
  y: number;
}

/** Project a tile's *top corner* into local screen space. */
export function tileToScreen(gridX: number, gridY: number, level = 0): ScreenPoint {
  return {
    x: (gridX - gridY) * HALF_TILE_WIDTH,
    y: (gridX + gridY) * HALF_TILE_HEIGHT - level * ELEVATION_HEIGHT,
  };
}

/** Project the *centre* of a tile (where an agent's feet rest). */
export function tileCenterToScreen(gridX: number, gridY: number, level = 0): ScreenPoint {
  const corner = tileToScreen(gridX, gridY, level);
  return { x: corner.x, y: corner.y + HALF_TILE_HEIGHT };
}

/**
 * Inverse projection: turn a local screen position back into fractional tile
 * coordinates. Elevation is ignored — callers resolve height from the grid.
 */
export function screenToTile(screenX: number, screenY: number): TilePoint {
  const normalizedX = screenX / HALF_TILE_WIDTH;
  const normalizedY = screenY / HALF_TILE_HEIGHT;
  return { x: (normalizedY + normalizedX) / 2, y: (normalizedY - normalizedX) / 2 };
}

/**
 * Layer biases keep entities that share a tile in a stable front-to-back order
 * without disturbing the dominant `gridX + gridY` ordering.
 */
export const DEPTH_TILE_SCALE = 16;
export const LAYER_FLOOR = 0;
export const LAYER_DECAL = 1;
export const LAYER_WALL = 2;
export const LAYER_FURNITURE = 3;
export const LAYER_AGENT = 6;

/** Painter's-order depth for a point at `(gridX, gridY)` and elevation `level`. */
export function depthAt(gridX: number, gridY: number, level = 0, layer = 0): number {
  return (gridX + gridY) * DEPTH_TILE_SCALE + level * (DEPTH_TILE_SCALE / 2) + layer;
}

/** Linear interpolation between two scalars. */
export function lerp(from: number, to: number, amount: number): number {
  return from + (to - from) * amount;
}

/** Euclidean distance between two tile points. */
export function tileDistance(from: TilePoint, to: TilePoint): number {
  return Math.hypot(to.x - from.x, to.y - from.y);
}
