/**
 * The four built-in room types.
 *
 * Every room is expressed as data — a tile matrix plus furniture placements —
 * and wrapped in a thin {@link BaseRoom} subclass. Walls are authored only on
 * the back (north + west) edges so the camera looks into an open interior, the
 * classic isometric room read.
 */
import { BaseRoom } from './BaseRoom';
import type { TextureFactory } from './textures';
import {
  type Facing,
  type FurnitureConfig,
  type InteractionPoint,
  type RoomDefinition,
  type RoomPalette,
  TileCode,
} from './types';

// ---- Matrix authoring helpers ----------------------------------------------

function floorGrid(columns: number, rows: number): Array<Array<TileCode>> {
  const matrix: Array<Array<TileCode>> = [];
  for (let row = 0; row < rows; row++) {
    matrix.push(new Array<TileCode>(columns).fill(TileCode.Floor));
  }
  return matrix;
}

function addBackWalls(matrix: Array<Array<TileCode>>): void {
  for (let column = 0; column < (matrix[0]?.length ?? 0); column++) {
    matrix[0]![column] = TileCode.Wall;
  }
  for (let row = 0; row < matrix.length; row++) {
    matrix[row]![0] = TileCode.Wall;
  }
}

function fillRectangle(
  matrix: Array<Array<TileCode>>,
  tileX: number,
  tileY: number,
  width: number,
  height: number,
  code: TileCode
): void {
  for (let row = tileY; row < tileY + height; row++) {
    for (let column = tileX; column < tileX + width; column++) {
      if (matrix[row]?.[column] !== undefined) {
        matrix[row]![column] = code;
      }
    }
  }
}

function putTile(
  matrix: Array<Array<TileCode>>,
  tileX: number,
  tileY: number,
  code: TileCode
): void {
  const row = matrix[tileY];
  if (row && row[tileX] !== undefined) {
    row[tileX] = code;
  }
}

function chair(
  tileX: number,
  tileY: number,
  facing: Facing,
  level = 0,
  tint?: number
): FurnitureConfig {
  const station: InteractionPoint = {
    tileOffsetX: 0,
    tileOffsetY: 0,
    action: 'sit',
    facing,
    seatDropY: 6,
  };
  return { kind: 'chair', tileX, tileY, level, tint, interactionPoints: [station] };
}

// ---- Palettes ---------------------------------------------------------------

const POKER_PALETTE: RoomPalette = {
  background: 0x0c1810,
  floorTop: 0x356046,
  floorSide: 0x244432,
  wall: 0x4a3526,
  dais: 0x3c5a44,
  accent: 0x34d399,
};

const COURT_PALETTE: RoomPalette = {
  background: 0x15121c,
  floorTop: 0xcdc7ba,
  floorSide: 0x9c9788,
  wall: 0x4a3326,
  dais: 0x7c5e46,
  accent: 0xa78bfa,
};

const OFFICE_PALETTE: RoomPalette = {
  background: 0x121620,
  floorTop: 0x8f98a6,
  floorSide: 0x6a727f,
  wall: 0x5b6270,
  dais: 0x768091,
  accent: 0x60a5fa,
};

const HOME_PALETTE: RoomPalette = {
  background: 0x1a1320,
  floorTop: 0xb98a63,
  floorSide: 0x8f6a49,
  wall: 0x9a8f7a,
  dais: 0xb98a63,
  accent: 0xfbbf24,
};

const OUTSIDE_PALETTE: RoomPalette = {
  background: 0x223047,
  floorTop: 0x5f9450,
  floorSide: 0x3f6e35,
  wall: 0x6b7280,
  dais: 0x7a8a6a,
  accent: 0x7fc8a9,
};

// ---- Poker table room -------------------------------------------------------

function pokerDefinition(): RoomDefinition {
  const matrix = floorGrid(12, 11);
  addBackWalls(matrix);
  const furniture: Array<FurnitureConfig> = [
    { kind: 'pokerTable', tileX: 5, tileY: 4 },
    // Eight seats facing inward toward the felt.
    chair(5, 3, 'right'),
    chair(7, 3, 'left'),
    chair(5, 6, 'right'),
    chair(7, 6, 'left'),
    chair(4, 4, 'right'),
    chair(4, 5, 'right'),
    chair(8, 4, 'left'),
    chair(8, 5, 'left'),
    // A little bar with stools in front of the counter.
    { kind: 'barCounter', tileX: 2, tileY: 8 },
    { kind: 'stool', tileX: 2, tileY: 9 },
    { kind: 'stool', tileX: 3, tileY: 9 },
    { kind: 'stool', tileX: 4, tileY: 9 },
    { kind: 'lamp', tileX: 1, tileY: 1 },
    { kind: 'fern', tileX: 10, tileY: 1 },
    { kind: 'trophy', tileX: 1, tileY: 9 },
    { kind: 'plant', tileX: 10, tileY: 9 },
  ];
  return {
    key: 'poker',
    name: 'Poker Table',
    description: 'A felt table ringed with eight seats for a full table of agents.',
    matrix,
    palette: POKER_PALETTE,
    furniture,
    spawnTile: { x: 6, y: 9 },
  };
}

// ---- Court house ------------------------------------------------------------

function courtDefinition(): RoomDefinition {
  const matrix = floorGrid(12, 14);
  addBackWalls(matrix);
  // Raised dais tier across the top-centre for the bench.
  fillRectangle(matrix, 3, 1, 6, 2, TileCode.Dais);
  const furniture: Array<FurnitureConfig> = [
    { kind: 'judgeBench', tileX: 5, tileY: 1, level: 1 },
    chair(4, 1, 'right', 1),
    { kind: 'witnessStand', tileX: 8, tileY: 2, level: 1 },
    // Counsel tables facing the bench.
    { kind: 'courtTable', tileX: 3, tileY: 9 },
    chair(3, 10, 'right'),
    chair(4, 10, 'right'),
    { kind: 'courtTable', tileX: 7, tileY: 9 },
    chair(7, 10, 'left'),
    chair(8, 10, 'left'),
    // Jury box along the west wall.
    chair(1, 4, 'right'),
    chair(1, 5, 'right'),
    chair(1, 6, 'right'),
    chair(2, 4, 'right'),
    chair(2, 5, 'right'),
    chair(2, 6, 'right'),
    // Public gallery at the back.
    chair(3, 12, 'right'),
    chair(4, 12, 'right'),
    chair(7, 12, 'left'),
    chair(8, 12, 'left'),
    { kind: 'painting', tileX: 10, tileY: 1 },
    { kind: 'crate', tileX: 9, tileY: 6 },
    { kind: 'lamp', tileX: 1, tileY: 12 },
    { kind: 'fern', tileX: 10, tileY: 12 },
  ];
  return {
    key: 'court',
    name: 'Court House',
    description: "A raised judge's bench, a jury box, counsel tables and a public gallery.",
    matrix,
    palette: COURT_PALETTE,
    furniture,
    spawnTile: { x: 6, y: 12 },
  };
}

// ---- Office -----------------------------------------------------------------

function officeDefinition(): RoomDefinition {
  const matrix = floorGrid(13, 12);
  addBackWalls(matrix);
  // Cubicle partition walls.
  const partitions: Array<[number, number]> = [
    [4, 2],
    [4, 3],
    [8, 2],
    [8, 3],
    [4, 6],
    [4, 7],
    [8, 6],
    [8, 7],
  ];
  for (const [column, row] of partitions) {
    putTile(matrix, column, row, TileCode.Partition);
  }
  const furniture: Array<FurnitureConfig> = [
    // Top row of cubicles.
    { kind: 'desk', tileX: 1, tileY: 2 },
    chair(1, 3, 'right', 0, 0x556070),
    { kind: 'desk', tileX: 5, tileY: 2 },
    chair(5, 3, 'right', 0, 0x556070),
    { kind: 'desk', tileX: 9, tileY: 2 },
    chair(9, 3, 'right', 0, 0x556070),
    // Bottom row of cubicles.
    { kind: 'desk', tileX: 1, tileY: 6 },
    chair(1, 7, 'right', 0, 0x556070),
    { kind: 'desk', tileX: 5, tileY: 6 },
    chair(5, 7, 'right', 0, 0x556070),
    { kind: 'desk', tileX: 9, tileY: 6 },
    chair(9, 7, 'right', 0, 0x556070),
    {
      kind: 'whiteboard',
      tileX: 11,
      tileY: 1,
      interactionPoints: [{ tileOffsetX: 0, tileOffsetY: 1, action: 'inspect', facing: 'left' }],
    },
    { kind: 'bookshelf', tileX: 12, tileY: 4 },
    { kind: 'bookshelf', tileX: 12, tileY: 5 },
    { kind: 'plant', tileX: 3, tileY: 10 },
    { kind: 'plant', tileX: 10, tileY: 10 },
    { kind: 'lamp', tileX: 12, tileY: 9 },
    { kind: 'crate', tileX: 12, tileY: 10 },
    { kind: 'fern', tileX: 11, tileY: 10 },
    { kind: 'painting', tileX: 1, tileY: 1 },
  ];
  return {
    key: 'office',
    name: 'Office',
    description: 'Cubicle desks, a whiteboard and bookshelves for heads-down agent work.',
    matrix,
    palette: OFFICE_PALETTE,
    furniture,
    spawnTile: { x: 6, y: 10 },
  };
}

// ---- Home -------------------------------------------------------------------

function homeDefinition(): RoomDefinition {
  const matrix = floorGrid(12, 11);
  addBackWalls(matrix);
  const furniture: Array<FurnitureConfig> = [
    { kind: 'rug', tileX: 3, tileY: 4, footprintWidth: 4, footprintHeight: 3, tint: 0x7a4f5e },
    { kind: 'tvStand', tileX: 4, tileY: 2 },
    { kind: 'couch', tileX: 4, tileY: 6, tint: 0x4a6ea0 },
    { kind: 'coffeeTable', tileX: 5, tileY: 4 },
    chair(7, 4, 'left', 0, 0x8a5a6a),
    { kind: 'bed', tileX: 9, tileY: 7 },
    { kind: 'bookshelf', tileX: 1, tileY: 2 },
    { kind: 'bookshelf', tileX: 1, tileY: 3 },
    { kind: 'door', tileX: 8, tileY: 1 },
    { kind: 'plant', tileX: 10, tileY: 1 },
    { kind: 'plant', tileX: 2, tileY: 9 },
    { kind: 'lamp', tileX: 2, tileY: 6 },
    { kind: 'fern', tileX: 10, tileY: 9 },
    { kind: 'painting', tileX: 6, tileY: 1 },
    { kind: 'stool', tileX: 7, tileY: 6 },
  ];
  return {
    key: 'home',
    name: 'Home',
    description: 'A cosy lounge with couches, a rug, a bed and a custom door.',
    matrix,
    palette: HOME_PALETTE,
    furniture,
    spawnTile: { x: 6, y: 9 },
  };
}

// ---- Outside world ----------------------------------------------------------

const CORNER_BUILDINGS = ['tower', 'glassTower']; // 2x2 — fit the narrow corners
const TOP_BUILDINGS = ['house', 'shop', 'cafe', 'apartment']; // 3 wide
const BOTTOM_BUILDINGS = ['shop', 'cafe', 'apartment']; // 3x2 (no 3-tall house)

/**
 * Procedurally lay out a multi-block city: a grid of streets with sidewalks,
 * each block packed with varied buildings, plus cars, lamps, trees, and a
 * central park. Deterministic (fixed seed) so the city is stable.
 */
function outsideDefinition(): RoomDefinition {
  const blocks = 5;
  const pitch = 12; // tiles between successive road bands
  const road = 2;
  const size = blocks * pitch + road; // 62
  const matrix = floorGrid(size, size);

  // Lay the road grid.
  for (let band = 0; band <= blocks; band++) {
    const at = band * pitch;
    fillRectangle(matrix, at, 0, road, size, TileCode.Road);
    fillRectangle(matrix, 0, at, size, road, TileCode.Road);
  }
  // Any grass tile touching a road becomes a sidewalk.
  const isRoad = (column: number, row: number): boolean => matrix[row]?.[column] === TileCode.Road;
  for (let row = 0; row < size; row++) {
    for (let column = 0; column < size; column++) {
      if (matrix[row]![column] !== TileCode.Floor) {
        continue;
      }
      if (
        isRoad(column - 1, row) ||
        isRoad(column + 1, row) ||
        isRoad(column, row - 1) ||
        isRoad(column, row + 1)
      ) {
        matrix[row]![column] = TileCode.Pavement;
      }
    }
  }

  let seed = 0x1357acef;
  const random = (): number => {
    seed = (seed * 1664525 + 1013904223) >>> 0;
    return seed / 0x100000000;
  };
  const pick = (list: ReadonlyArray<string>): string => list[Math.floor(random() * list.length)]!;

  const furniture: Array<FurnitureConfig> = [];
  const parkBlock = Math.floor(blocks / 2);

  for (let blockY = 0; blockY < blocks; blockY++) {
    for (let blockX = 0; blockX < blocks; blockX++) {
      const ox = blockX * pitch + 3; // 8x8 block interior origin
      const oy = blockY * pitch + 3;
      // A street lamp on the block's north-west sidewalk corner.
      furniture.push({ kind: 'lamp', tileX: ox - 1, tileY: oy - 1 });
      furniture.push({ kind: 'fern', tileX: ox - 1, tileY: oy + 4 });

      if (blockX === parkBlock && blockY === parkBlock) {
        // The central park.
        furniture.push({ kind: 'fountain', tileX: ox + 3, tileY: oy + 3 });
        furniture.push(chair(ox + 3, oy + 5, 'right', 0, 0x6b7280));
        furniture.push(chair(ox + 4, oy + 5, 'left', 0, 0x6b7280));
        furniture.push({ kind: 'fern', tileX: ox + 1, tileY: oy + 1 });
        furniture.push({ kind: 'fern', tileX: ox + 6, tileY: oy + 1 });
        furniture.push({ kind: 'fern', tileX: ox + 1, tileY: oy + 6 });
        furniture.push({ kind: 'fern', tileX: ox + 6, tileY: oy + 6 });
        continue;
      }

      // Four buildings around the block, leaving a yard in the middle.
      furniture.push({ kind: pick(TOP_BUILDINGS), tileX: ox, tileY: oy });
      furniture.push({ kind: pick(CORNER_BUILDINGS), tileX: ox + 6, tileY: oy });
      furniture.push({ kind: pick(BOTTOM_BUILDINGS), tileX: ox, tileY: oy + 6 });
      furniture.push({ kind: pick(CORNER_BUILDINGS), tileX: ox + 6, tileY: oy + 6 });
      furniture.push({ kind: 'fern', tileX: ox + 4, tileY: oy + 4 });
    }
  }

  // Moving traffic (live cars) is spawned by the world controller, not baked
  // into the static furniture here.

  // Dashed lane markings down each road centre (skipping the intersections).
  for (let band = 0; band <= blocks; band++) {
    const at = band * pitch;
    for (let index = 1; index < size; index += 2) {
      if (index % pitch >= road) {
        furniture.push({ kind: 'roadDash', tileX: at, tileY: index });
        furniture.push({ kind: 'roadDash', tileX: index, tileY: at });
      }
    }
  }

  return {
    key: 'outside',
    name: 'Outside World',
    description:
      'A sprawling pixel city — a grid of streets, cars, towers and a central park. Drag to explore.',
    matrix,
    palette: OUTSIDE_PALETTE,
    furniture,
    spawnTile: { x: parkBlock * pitch + 6, y: parkBlock * pitch },
    topMargin: 230,
  };
}

// ---- Concrete room subclasses ----------------------------------------------

export class PokerTableRoom extends BaseRoom {
  public constructor(factory: TextureFactory) {
    super(pokerDefinition(), factory);
  }
}

export class CourtHouseRoom extends BaseRoom {
  public constructor(factory: TextureFactory) {
    super(courtDefinition(), factory);
  }
}

export class OfficeRoom extends BaseRoom {
  public constructor(factory: TextureFactory) {
    super(officeDefinition(), factory);
  }
}

export class HomeRoom extends BaseRoom {
  public constructor(factory: TextureFactory) {
    super(homeDefinition(), factory);
  }
}

export class OutsideWorldRoom extends BaseRoom {
  public constructor(factory: TextureFactory) {
    super(outsideDefinition(), factory);
  }
}

export interface RoomEntry {
  key: string;
  name: string;
  description: string;
  create: (factory: TextureFactory) => BaseRoom;
}

export const ROOM_REGISTRY: Array<RoomEntry> = [
  {
    key: 'poker',
    name: 'Poker',
    description: 'Eight seats around a felt table.',
    create: factory => new PokerTableRoom(factory),
  },
  {
    key: 'court',
    name: 'Court',
    description: 'Raised bench, jury box and gallery.',
    create: factory => new CourtHouseRoom(factory),
  },
  {
    key: 'office',
    name: 'Office',
    description: 'Cubicles, desks and a whiteboard.',
    create: factory => new OfficeRoom(factory),
  },
  {
    key: 'home',
    name: 'Home',
    description: 'A cosy lounge with couches and a rug.',
    create: factory => new HomeRoom(factory),
  },
  {
    key: 'outside',
    name: 'World',
    description: 'A large open plaza ringed with buildings.',
    create: factory => new OutsideWorldRoom(factory),
  },
];
