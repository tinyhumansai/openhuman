/**
 * Procedural texture factory.
 *
 * Every visual in the world is baked once, in greyscale, into a small shared
 * texture and then reused via `tint`. That keeps GPU memory tiny (a handful of
 * textures regardless of how many agents or chairs exist) and is what lets us
 * recolour entities for free. All textures are generated at resolution 1 with
 * `nearest` filtering so they stay crisp and chunky when the world scales up.
 */
import { Graphics, type Renderer, type Texture } from 'pixi.js';

import { HALF_TILE_HEIGHT, HALF_TILE_WIDTH, TILE_HEIGHT, TILE_WIDTH } from './geometry';

/** A baked texture plus the pivot that re-aligns its origin to the tile anchor. */
export interface BakedTexture {
  texture: Texture;
  anchorX: number;
  anchorY: number;
}

const SHADE_TOP = 0xffffff;
const SHADE_LEFT = 0xb6b6b6;
const SHADE_RIGHT = 0x8d8d8d;
const EDGE_COLOR = 0x05060a;

function diamondPoints(
  footprintWidth: number,
  footprintHeight: number,
  lift: number
): Array<number> {
  const north = [0, -lift];
  const east = [footprintWidth * HALF_TILE_WIDTH, footprintWidth * HALF_TILE_HEIGHT - lift];
  const south = [
    (footprintWidth - footprintHeight) * HALF_TILE_WIDTH,
    (footprintWidth + footprintHeight) * HALF_TILE_HEIGHT - lift,
  ];
  const west = [-footprintHeight * HALF_TILE_WIDTH, footprintHeight * HALF_TILE_HEIGHT - lift];
  return [...north, ...east, ...south, ...west];
}

export class TextureFactory {
  private readonly renderer: Renderer;
  private readonly cache = new Map<string, BakedTexture>();

  public constructor(renderer: Renderer) {
    this.renderer = renderer;
  }

  /** Bake a `Graphics` whose local origin is the tile anchor point. */
  private bake(key: string, graphics: Graphics): BakedTexture {
    const existing = this.cache.get(key);
    if (existing) {
      graphics.destroy();
      return existing;
    }
    const bounds = graphics.getLocalBounds();
    const texture = this.renderer.generateTexture({
      target: graphics,
      resolution: 1,
      antialias: false,
    });
    texture.source.scaleMode = 'nearest';
    const baked: BakedTexture = { texture, anchorX: -bounds.minX, anchorY: -bounds.minY };
    graphics.destroy();
    this.cache.set(key, baked);
    return baked;
  }

  /** A flat floor diamond: a darker grout border, a bright surface and a sheen. */
  public floorTile(): BakedTexture {
    const graphics = new Graphics();
    // Grout/border underneath (reads darker once tinted).
    graphics.poly(diamondPoints(1, 1, 0)).fill({ color: 0xb2b2b2 });
    // Bright inset surface.
    const inset = 0.1;
    const surface = [
      0,
      inset * TILE_HEIGHT,
      TILE_WIDTH / 2 - inset * TILE_WIDTH,
      TILE_HEIGHT / 2,
      0,
      TILE_HEIGHT - inset * TILE_HEIGHT,
      -(TILE_WIDTH / 2 - inset * TILE_WIDTH),
      TILE_HEIGHT / 2,
    ];
    graphics.poly(surface).fill({ color: SHADE_TOP });
    // A soft specular sheen toward the north corner.
    graphics.poly([0, 3, 12, 9, 0, 15, -12, 9]).fill({ color: 0xffffff, alpha: 0.16 });
    graphics
      .poly(diamondPoints(1, 1, 0))
      .stroke({ color: EDGE_COLOR, width: 1, alpha: 0.14, alignment: 0 });
    return this.bake('floor', graphics);
  }

  /** Asphalt road tile — a dark surface with a worn sheen and a curb seam. */
  public roadTile(): BakedTexture {
    const graphics = new Graphics();
    graphics.poly(diamondPoints(1, 1, 0)).fill({ color: 0x2b2e36 });
    const inset = 0.07;
    graphics
      .poly([
        0,
        inset * TILE_HEIGHT,
        TILE_WIDTH / 2 - inset * TILE_WIDTH,
        TILE_HEIGHT / 2,
        0,
        TILE_HEIGHT - inset * TILE_HEIGHT,
        -(TILE_WIDTH / 2 - inset * TILE_WIDTH),
        TILE_HEIGHT / 2,
      ])
      .fill({ color: 0x3c4049 });
    graphics.poly([0, 5, 9, 9, 0, 13, -9, 9]).fill({ color: 0xffffff, alpha: 0.04 });
    return this.bake('road', graphics);
  }

  /** A short dashed lane marking, laid down the centre of a road. */
  public roadDash(): BakedTexture {
    const graphics = new Graphics();
    graphics.poly([0, 11, 6, 14.5, 0, 18, -6, 14.5]).fill({ color: 0xe6c34a, alpha: 0.85 });
    return this.bake('road-dash', graphics);
  }

  /** A flat decorative diamond (rugs, mats) spanning a footprint. */
  public decal(footprintWidth: number, footprintHeight: number): BakedTexture {
    const key = `decal:${footprintWidth}x${footprintHeight}`;
    const graphics = new Graphics();
    graphics
      .poly(diamondPoints(footprintWidth, footprintHeight, 0))
      .fill({ color: SHADE_TOP })
      .stroke({ color: EDGE_COLOR, width: 2, alpha: 0.25 });
    return this.bake(key, graphics);
  }

  /**
   * A three-tone isometric cuboid spanning `footprintWidth x footprintHeight`
   * tiles and rising `height` pixels. The workhorse behind walls, tables,
   * benches, desks and the courthouse dais.
   */
  public cuboid(
    footprintWidth: number,
    footprintHeight: number,
    height: number,
    key = `cuboid:${footprintWidth}x${footprintHeight}x${height}`
  ): BakedTexture {
    const graphics = new Graphics();
    const top = diamondPoints(footprintWidth, footprintHeight, height);
    // Top corners: [north, east, south, west].
    const eastX = top[2]!;
    const eastY = top[3]!;
    const southX = top[4]!;
    const southY = top[5]!;
    const westX = top[6]!;
    const westY = top[7]!;
    // Right face (east -> south) dropped to ground.
    graphics
      .poly([eastX, eastY, southX, southY, southX, southY + height, eastX, eastY + height])
      .fill({ color: SHADE_RIGHT })
      .stroke({ color: EDGE_COLOR, width: 1, alpha: 0.3 });
    // Left face (south -> west) dropped to ground.
    graphics
      .poly([southX, southY, westX, westY, westX, westY + height, southX, southY + height])
      .fill({ color: SHADE_LEFT })
      .stroke({ color: EDGE_COLOR, width: 1, alpha: 0.3 });
    // Soft bottom shading on the side faces for grounded depth.
    const band = height * 0.62;
    graphics
      .poly([
        eastX,
        eastY + band,
        southX,
        southY + band,
        southX,
        southY + height,
        eastX,
        eastY + height,
      ])
      .fill({ color: 0x000000, alpha: 0.16 });
    graphics
      .poly([
        southX,
        southY + band,
        westX,
        westY + band,
        westX,
        westY + height,
        southX,
        southY + height,
      ])
      .fill({ color: 0x000000, alpha: 0.12 });
    // Top face last so the seams sit cleanly on top.
    graphics
      .poly(top)
      .fill({ color: SHADE_TOP })
      .stroke({ color: EDGE_COLOR, width: 1, alpha: 0.35 });
    // A faint highlight along the north top edge.
    graphics
      .poly([top[0]!, top[1]!, top[2]!, top[3]!, top[6]!, top[7]!])
      .fill({ color: 0xffffff, alpha: 0.08 });
    return this.bake(key, graphics);
  }

  /**
   * A tall, detailed wall block: shaded faces with a cornice cap, a chair-rail
   * trim, a lighter wainscot panel with vertical grooves, and a dark baseboard.
   * Everything is greyscale so the room palette's wall colour tints it whole.
   */
  public wallBlock(height: number): BakedTexture {
    const graphics = new Graphics();
    const top = diamondPoints(1, 1, height);
    const northX = top[0]!;
    const northY = top[1]!;
    const eastX = top[2]!;
    const eastY = top[3]!;
    const southX = top[4]!;
    const southY = top[5]!;
    const westX = top[6]!;
    const westY = top[7]!;

    // A horizontal band across a face, expressed as height fractions (0 = top).
    const rightBand = (from: number, to: number, color: number, alpha = 1): void => {
      graphics
        .poly([
          eastX,
          eastY + from * height,
          southX,
          southY + from * height,
          southX,
          southY + to * height,
          eastX,
          eastY + to * height,
        ])
        .fill({ color, alpha });
    };
    const leftBand = (from: number, to: number, color: number, alpha = 1): void => {
      graphics
        .poly([
          southX,
          southY + from * height,
          westX,
          westY + from * height,
          westX,
          westY + to * height,
          southX,
          southY + to * height,
        ])
        .fill({ color, alpha });
    };
    // A vertical groove line down a face at horizontal fraction `t`.
    const groove = (
      cornerAX: number,
      cornerAY: number,
      cornerBX: number,
      cornerBY: number,
      t: number
    ): void => {
      const x = cornerAX + (cornerBX - cornerAX) * t;
      const y = cornerAY + (cornerBY - cornerAY) * t;
      graphics
        .moveTo(x, y + 0.18 * height)
        .lineTo(x, y + 0.88 * height)
        .stroke({ color: 0x000000, width: 1, alpha: 0.16 });
    };

    // Right (east) face: three tiers split by mouldings, with a wainscot
    // and baseboard at the bottom and a cornice on top.
    rightBand(0, 1, 0x8d8d8d);
    rightBand(0.62, 0.9, 0x9b9b9b);
    rightBand(0.59, 0.62, 0x6c6c6c);
    rightBand(0.31, 0.34, 0x6c6c6c);
    rightBand(0.9, 1, 0x5d5d5d);
    rightBand(0, 0.05, 0xa8a8a8);
    groove(eastX, eastY, southX, southY, 0.34);
    groove(eastX, eastY, southX, southY, 0.67);
    graphics
      .poly([eastX, eastY, southX, southY, southX, southY + height, eastX, eastY + height])
      .stroke({ color: EDGE_COLOR, width: 1, alpha: 0.3 });

    // Left (west) face, a shade lighter overall.
    leftBand(0, 1, 0xb6b6b6);
    leftBand(0.62, 0.9, 0xc4c4c4);
    leftBand(0.59, 0.62, 0x979797);
    leftBand(0.31, 0.34, 0x979797);
    leftBand(0.9, 1, 0x868686);
    leftBand(0, 0.05, 0xd0d0d0);
    groove(southX, southY, westX, westY, 0.34);
    groove(southX, southY, westX, westY, 0.67);
    graphics
      .poly([southX, southY, westX, westY, westX, westY + height, southX, southY + height])
      .stroke({ color: EDGE_COLOR, width: 1, alpha: 0.3 });

    // Top cap with a brighter inner cornice.
    graphics
      .poly(top)
      .fill({ color: 0xd2d2d2 })
      .stroke({ color: EDGE_COLOR, width: 1, alpha: 0.35 });
    const capInset = 0.16;
    graphics
      .poly([
        northX,
        northY + capInset * TILE_HEIGHT,
        eastX - capInset * TILE_WIDTH,
        eastY,
        southX,
        southY - capInset * TILE_HEIGHT,
        westX + capInset * TILE_WIDTH,
        westY,
      ])
      .fill({ color: 0xe6e6e6 });
    return this.bake(`wall:${height}`, graphics);
  }

  /**
   * The detail overlay for a building: a coloured roof, a grid of lit windows
   * on both visible faces, and a door. Baked untinted so it keeps its own
   * colours over a tinted body cuboid (much like the agent face over a body).
   */
  public buildingDetail(
    footprintWidth: number,
    footprintHeight: number,
    height: number,
    options: {
      windowRows: number;
      windowColumns: number;
      windowColor: number;
      roofColor: number;
      doorColor: number;
    }
  ): BakedTexture {
    const { windowRows, windowColumns, windowColor, roofColor, doorColor } = options;
    const graphics = new Graphics();
    const top = diamondPoints(footprintWidth, footprintHeight, height);
    const northX = top[0]!;
    const northY = top[1]!;
    const eastX = top[2]!;
    const eastY = top[3]!;
    const southX = top[4]!;
    const southY = top[5]!;
    const westX = top[6]!;
    const westY = top[7]!;

    const facePoint = (
      aX: number,
      aY: number,
      bX: number,
      bY: number,
      u: number,
      v: number
    ): Array<number> => [aX + (bX - aX) * u, aY + (bY - aY) * u + v * height];

    const drawFace = (aX: number, aY: number, bX: number, bY: number): void => {
      // Eave shadow band under the roof.
      graphics
        .poly([
          ...facePoint(aX, aY, bX, bY, 0, 0),
          ...facePoint(aX, aY, bX, bY, 1, 0),
          ...facePoint(aX, aY, bX, bY, 1, 0.06),
          ...facePoint(aX, aY, bX, bY, 0, 0.06),
        ])
        .fill({ color: 0x000000, alpha: 0.2 });
      // Foundation band along the base.
      graphics
        .poly([
          ...facePoint(aX, aY, bX, bY, 0, 0.9),
          ...facePoint(aX, aY, bX, bY, 1, 0.9),
          ...facePoint(aX, aY, bX, bY, 1, 1),
          ...facePoint(aX, aY, bX, bY, 0, 1),
        ])
        .fill({ color: 0x000000, alpha: 0.24 });
      // Window grid.
      const bandTop = 0.14;
      const bandBottom = windowRows >= 4 ? 0.86 : 0.6;
      const gapV = 0.045;
      const cellV = (bandBottom - bandTop - (windowRows - 1) * gapV) / windowRows;
      const padU = 0.16;
      const gapU = 0.07;
      const cellU = (1 - 2 * padU - (windowColumns - 1) * gapU) / windowColumns;
      for (let row = 0; row < windowRows; row++) {
        const v1 = bandTop + row * (cellV + gapV);
        const v2 = v1 + cellV;
        for (let column = 0; column < windowColumns; column++) {
          const u1 = padU + column * (cellU + gapU);
          const u2 = u1 + cellU;
          graphics
            .poly([
              ...facePoint(aX, aY, bX, bY, u1, v1),
              ...facePoint(aX, aY, bX, bY, u2, v1),
              ...facePoint(aX, aY, bX, bY, u2, v2),
              ...facePoint(aX, aY, bX, bY, u1, v2),
            ])
            .fill({ color: 0x161a24 });
          const mu = 0.014;
          const mv = cellV * 0.16;
          graphics
            .poly([
              ...facePoint(aX, aY, bX, bY, u1 + mu, v1 + mv),
              ...facePoint(aX, aY, bX, bY, u2 - mu, v1 + mv),
              ...facePoint(aX, aY, bX, bY, u2 - mu, v2 - mv),
              ...facePoint(aX, aY, bX, bY, u1 + mu, v2 - mv),
            ])
            .fill({ color: windowColor });
        }
      }
    };
    drawFace(eastX, eastY, southX, southY);
    drawFace(southX, southY, westX, westY);

    // A door near the south corner of the right face.
    graphics
      .poly([
        ...facePoint(eastX, eastY, southX, southY, 0.44, 0.66),
        ...facePoint(eastX, eastY, southX, southY, 0.6, 0.66),
        ...facePoint(eastX, eastY, southX, southY, 0.6, 0.97),
        ...facePoint(eastX, eastY, southX, southY, 0.44, 0.97),
      ])
      .fill({ color: doorColor });

    // Roof cap with a soft highlight.
    graphics
      .poly(top)
      .fill({ color: roofColor })
      .stroke({ color: EDGE_COLOR, width: 1, alpha: 0.35 });
    const centerX = (northX + eastX + southX + westX) / 4;
    const centerY = (northY + eastY + southY + westY) / 4;
    const inset = (x: number, y: number): Array<number> => [
      x + (centerX - x) * 0.2,
      y + (centerY - y) * 0.2,
    ];
    graphics
      .poly([
        ...inset(northX, northY),
        ...inset(eastX, eastY),
        ...inset(southX, southY),
        ...inset(westX, westY),
      ])
      .fill({ color: 0xffffff, alpha: 0.12 });
    return this.bake(
      `building:${footprintWidth}x${footprintHeight}x${height}:${windowRows}x${windowColumns}:${windowColor}:${roofColor}:${doorColor}`,
      graphics
    );
  }

  /** A small chair: a seat block with a low back, facing the camera. */
  public chair(): BakedTexture {
    const graphics = new Graphics();
    const seat = diamondPoints(0.62, 0.62, 12);
    const eastX = seat[2]!;
    const eastY = seat[3]!;
    const southX = seat[4]!;
    const southY = seat[5]!;
    const westX = seat[6]!;
    const westY = seat[7]!;
    const northX = seat[0]!;
    const northY = seat[1]!;
    // Seat sides.
    graphics
      .poly([eastX, eastY, southX, southY, southX, southY + 12, eastX, eastY + 12])
      .fill({ color: SHADE_RIGHT });
    graphics
      .poly([southX, southY, westX, westY, westX, westY + 12, southX, southY + 12])
      .fill({ color: SHADE_LEFT });
    graphics
      .poly(seat)
      .fill({ color: SHADE_TOP })
      .stroke({ color: EDGE_COLOR, width: 1, alpha: 0.3 });
    // Back rest rising from the north edge.
    graphics
      .poly([northX, northY, eastX, eastY, eastX, eastY - 18, northX, northY - 18])
      .fill({ color: SHADE_LEFT });
    graphics
      .poly([westX, westY, northX, northY, northX, northY - 18, westX, westY - 18])
      .fill({ color: SHADE_RIGHT });
    return this.bake('chair', graphics);
  }

  /**
   * A soft rounded character body — feet at local (0, 0), head above. Drawn in
   * greyscale so the per-agent `tint` colours the whole figure cohesively. The
   * face is a separate untinted overlay (see {@link TextureFactory.agentFace}).
   */
  public agentBody(): BakedTexture {
    const graphics = new Graphics();
    // Stubby arms tucked behind the torso.
    graphics.ellipse(-12, -15, 3.6, 7).fill({ color: 0xe2e2e2 });
    graphics.ellipse(12, -15, 3.6, 7).fill({ color: 0xe2e2e2 });
    // Two little feet.
    graphics.ellipse(-6, -2, 5, 3.4).fill({ color: 0xbcbcbc });
    graphics.ellipse(6, -2, 5, 3.4).fill({ color: 0xbcbcbc });
    // Torso capsule with a clean outline.
    graphics
      .roundRect(-13, -39, 26, 39, 12)
      .fill({ color: SHADE_TOP })
      .stroke({ color: 0x1a1d29, width: 1.5, alpha: 0.22, alignment: 0 });
    // Rounded belly shade and a soft chest highlight for volume.
    graphics.ellipse(0, -8, 11, 8).fill({ color: 0xd2d2d2, alpha: 0.55 });
    graphics.ellipse(-4, -29, 6, 7).fill({ color: 0xffffff, alpha: 0.5 });
    return this.bake('agent', graphics);
  }

  /**
   * A head accessory, attached at the top-centre of the head (local origin).
   * Fabric kinds are drawn light so they can be tinted; identity kinds (crown,
   * glasses, headphones, flower) carry their own colours and are left untinted.
   */
  public accessory(kind: string): BakedTexture {
    const graphics = new Graphics();
    switch (kind) {
      case 'antenna':
        graphics.rect(-0.8, -8, 1.6, 8).fill({ color: 0xc8c8c8 });
        graphics
          .circle(0, -9, 2.6)
          .fill({ color: 0xf2f2f2 })
          .stroke({ color: 0x1a1d29, width: 1, alpha: 0.2 });
        break;
      case 'cap':
        graphics.ellipse(0, -3, 8.5, 5).fill({ color: 0xffffff });
        graphics.ellipse(5, 1, 8, 2.6).fill({ color: 0xe2e2e2 });
        break;
      case 'beanie':
        graphics.ellipse(0, -4, 8.5, 6).fill({ color: 0xffffff });
        graphics.rect(-8, -2, 16, 3.4).fill({ color: 0xdddddd });
        graphics.circle(0, -11, 2.6).fill({ color: 0xffffff });
        break;
      case 'party':
        graphics.poly([0, -16, -6, 0, 6, 0]).fill({ color: 0xffffff });
        graphics.circle(0, -16, 2.4).fill({ color: 0xffe9a8 });
        break;
      case 'bow':
        graphics.poly([-8, -4, -1, 0, -8, 4]).fill({ color: 0xffffff });
        graphics.poly([8, -4, 1, 0, 8, 4]).fill({ color: 0xffffff });
        graphics.circle(0, 0, 2).fill({ color: 0xe2e2e2 });
        break;
      case 'crown':
        graphics.poly([-8, 0, -8, -5, -4, -1, 0, -7, 4, -1, 8, -5, 8, 0]).fill({ color: 0xf5c542 });
        graphics.rect(-8, -1, 16, 2.4).fill({ color: 0xe0ad2e });
        graphics.circle(0, -6, 1.5).fill({ color: 0xff5d6c });
        break;
      case 'glasses':
        graphics.circle(-5, 12, 3.2).stroke({ color: 0x1a1d29, width: 1.6 });
        graphics.circle(5, 12, 3.2).stroke({ color: 0x1a1d29, width: 1.6 });
        graphics.rect(-1.8, 11.4, 3.6, 1.1).fill({ color: 0x1a1d29 });
        break;
      case 'headphones':
        graphics
          .moveTo(-9, 2)
          .lineTo(-9, -6)
          .lineTo(-5, -9)
          .lineTo(5, -9)
          .lineTo(9, -6)
          .lineTo(9, 2)
          .stroke({ color: 0x2a2d3a, width: 2.4 });
        graphics.roundRect(-11, -2, 4, 7, 2).fill({ color: 0x2a2d3a });
        graphics.roundRect(7, -2, 4, 7, 2).fill({ color: 0x2a2d3a });
        break;
      case 'flower':
        for (let petal = 0; petal < 5; petal++) {
          const angle = (petal * Math.PI * 2) / 5 - Math.PI / 2;
          graphics
            .circle(Math.cos(angle) * 4, -3 + Math.sin(angle) * 4, 2.4)
            .fill({ color: 0xff8fab });
        }
        graphics.circle(0, -3, 2).fill({ color: 0xffe08a });
        break;
      default:
        break;
    }
    return this.bake(`accessory:${kind}`, graphics);
  }

  /** Soft contact shadow sized to a footprint, grounding a furniture piece. */
  public contactShadow(footprintWidth: number, footprintHeight: number): BakedTexture {
    const key = `contact:${footprintWidth}x${footprintHeight}`;
    const graphics = new Graphics();
    const centerX = ((footprintWidth - footprintHeight) / 2) * TILE_WIDTH * 0.5;
    const centerY = ((footprintWidth + footprintHeight) / 2) * TILE_HEIGHT * 0.5;
    const radiusX = (footprintWidth + footprintHeight) * HALF_TILE_WIDTH * 0.46;
    const radiusY = (footprintWidth + footprintHeight) * HALF_TILE_HEIGHT * 0.5;
    graphics.ellipse(centerX, centerY, radiusX, radiusY).fill({ color: 0x000000, alpha: 0.1 });
    graphics
      .ellipse(centerX, centerY, radiusX * 0.7, radiusY * 0.7)
      .fill({ color: 0x000000, alpha: 0.12 });
    return this.bake(key, graphics);
  }

  /**
   * Facial features, baked once and left untinted so eyes, smile and blush
   * stay legible on top of any body colour. Pupils and mouth sit slightly to
   * one side so a horizontal flip reads as the agent changing which way it
   * faces.
   */
  public agentFace(): BakedTexture {
    const graphics = new Graphics();
    // Rosy cheeks.
    graphics.ellipse(-9, 3, 3.2, 2.1).fill({ color: 0xff9aa2, alpha: 0.5 });
    graphics.ellipse(9, 3, 3.2, 2.1).fill({ color: 0xff9aa2, alpha: 0.5 });
    // Eye whites.
    graphics.ellipse(-5, 0, 4.2, 5.2).fill({ color: 0xffffff });
    graphics.ellipse(5, 0, 4.2, 5.2).fill({ color: 0xffffff });
    // Pupils, nudged toward the facing direction.
    graphics.ellipse(-4.2, 1, 2.3, 3).fill({ color: 0x1a1d29 });
    graphics.ellipse(5.8, 1, 2.3, 3).fill({ color: 0x1a1d29 });
    // Catchlights.
    graphics.circle(-5.2, -0.6, 1).fill({ color: 0xffffff });
    graphics.circle(4.8, -0.6, 1).fill({ color: 0xffffff });
    // A gentle smile.
    graphics.ellipse(0.8, 7, 3.4, 1.7).fill({ color: 0x1a1d29, alpha: 0.85 });
    return this.bake('face', graphics);
  }

  /** A soft elliptical ground shadow, decoupled so it can fade independently. */
  public agentShadow(): BakedTexture {
    const graphics = new Graphics();
    graphics.ellipse(0, 0, 14, 7).fill({ color: 0x000000, alpha: 0.28 });
    return this.bake('shadow', graphics);
  }

  /** Rounded nine-slice background for chat bubbles. */
  public bubbleBackground(): BakedTexture {
    const graphics = new Graphics();
    graphics
      .roundRect(0, 0, 48, 48, 14)
      .fill({ color: 0xffffff })
      .stroke({ color: 0x10131c, width: 2, alpha: 0.12, alignment: 0 });
    return this.bake('bubble', graphics);
  }

  /** Little downward tail that points the bubble at the agent's head. */
  public bubbleTail(): BakedTexture {
    const graphics = new Graphics();
    graphics
      .poly([0, 0, 14, 0, 7, 9])
      .fill({ color: 0xffffff })
      .stroke({ color: 0x10131c, width: 1, alpha: 0.1 });
    return this.bake('bubble-tail', graphics);
  }

  public destroy(): void {
    for (const baked of this.cache.values()) {
      baked.texture.destroy(true);
    }
    this.cache.clear();
  }
}
