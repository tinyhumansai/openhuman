/**
 * A single agent in the world.
 *
 * Agents are reconciled, not driven: an external controller sets an
 * authoritative {@link AgentState} and the agent interpolates toward it. Motion
 * is pure linear interpolation between tile centres at a configurable speed, so
 * there are no teleports — an agent always slides along its resolved path. The
 * sprite is a single shared body texture, tinted, plus a soft ground shadow and
 * a bitmap nameplate.
 */
import { BitmapText, Container, Sprite } from 'pixi.js';

import { depthAt, LAYER_AGENT, lerp, tileCenterToScreen } from './geometry';
import type { BakedTexture } from './textures';
import type { AgentAction, Facing, WalkNode } from './types';

const DEFAULT_SPEED = 2.6;
const ARRIVAL_EPSILON = 0.0001;
// Agents render a little larger than their baked size so they read clearly
// against the tiles. Nearest filtering keeps the upscale crisp.
const AGENT_SCALE = 1.35;

interface AgentOptions {
  id: string;
  body: BakedTexture;
  face: BakedTexture;
  shadow: BakedTexture;
  accessory: BakedTexture;
  accessoryTint: number;
  fontName: string;
  tint: number;
  label: string;
  spawn: WalkNode;
}

export class Agent extends Container {
  public readonly agentId: string;

  private readonly figure = new Container();
  private readonly body: Sprite;
  private readonly face: Sprite;
  private readonly accessory: Sprite;
  private readonly shadow: Sprite;
  private readonly nameplate: BitmapText;

  private tileX: number;
  private tileY: number;
  private level: number;

  private readonly queue: Array<WalkNode> = [];
  private segmentFrom: WalkNode;
  private segmentTo: WalkNode;
  private segmentProgress = 1;
  private speed = DEFAULT_SPEED;

  private action: AgentAction = 'idle';
  private facing: Facing = 'right';
  private arrivalAction: AgentAction = 'idle';
  private arrivalFacing: Facing | null = null;
  private arrivalSeatDropY = 0;

  private seatDropY = 0;
  private bobPhase = 0;

  public constructor(options: AgentOptions) {
    super();
    this.agentId = options.id;
    this.tileX = options.spawn.x;
    this.tileY = options.spawn.y;
    this.level = options.spawn.level;
    this.segmentFrom = { ...options.spawn };
    this.segmentTo = { ...options.spawn };
    // Vary the idle bob so a crowd never breathes in lockstep.
    this.bobPhase = (options.spawn.x * 1.7 + options.spawn.y * 2.3) % (Math.PI * 2);

    this.shadow = new Sprite(options.shadow.texture);
    this.shadow.anchor.set(0.5, 0.5);
    this.shadow.position.set(0, -2);

    this.body = new Sprite(options.body.texture);
    this.body.pivot.set(options.body.anchorX, options.body.anchorY);
    this.body.tint = options.tint;

    // The face is an untinted overlay sitting on the head.
    this.face = new Sprite(options.face.texture);
    this.face.pivot.set(options.face.anchorX, options.face.anchorY);
    this.face.position.set(0, -27);

    // A head accessory (hat, glasses, antenna, ...) attached at the head top.
    this.accessory = new Sprite(options.accessory.texture);
    this.accessory.pivot.set(options.accessory.anchorX, options.accessory.anchorY);
    this.accessory.position.set(0, -38);
    this.accessory.tint = options.accessoryTint;

    // Body + face + accessory live in one figure so facing flips them together.
    this.figure.addChild(this.body, this.face, this.accessory);

    this.nameplate = new BitmapText({
      text: options.label,
      style: { fontFamily: options.fontName, fontSize: 11, align: 'center' },
    });
    this.nameplate.anchor.set(0.5, 1);

    this.addChild(this.shadow, this.figure, this.nameplate);
    this.syncScreenPosition();
  }

  public get currentTile(): WalkNode {
    return { x: this.tileX, y: this.tileY, level: this.level };
  }

  public get currentAction(): AgentAction {
    return this.action;
  }

  public get destinationTile(): WalkNode {
    return this.queue.at(-1) ?? this.segmentTo;
  }

  public setLabel(label: string): void {
    this.nameplate.text = label;
  }

  public setTint(tint: number): void {
    this.body.tint = tint;
  }

  public setSpeed(speed: number): void {
    this.speed = Math.max(0.4, speed);
  }

  /** Instantly relocate (used when a brand-new agent first appears). */
  public teleportTo(node: WalkNode): void {
    this.queue.length = 0;
    this.tileX = node.x;
    this.tileY = node.y;
    this.level = node.level;
    this.segmentFrom = { ...node };
    this.segmentTo = { ...node };
    this.segmentProgress = 1;
    this.action = 'idle';
    this.seatDropY = 0;
    this.syncScreenPosition();
  }

  /**
   * Walk along a resolved path, then settle into `arrivalAction`. An empty
   * path means the agent is already where it should be, so we only apply the
   * arrival pose.
   */
  public walkPath(
    path: Array<WalkNode>,
    arrivalAction: AgentAction,
    arrivalFacing: Facing | null,
    arrivalSeatDropY: number
  ): void {
    this.arrivalAction = arrivalAction;
    this.arrivalFacing = arrivalFacing;
    this.arrivalSeatDropY = arrivalSeatDropY;

    if (path.length === 0) {
      this.settleAtDestination();
      return;
    }

    this.queue.length = 0;
    this.queue.push(...path);
    this.beginNextSegment(true);
  }

  public tick(deltaSeconds: number): void {
    this.bobPhase += deltaSeconds * (this.action === 'walking' ? 9 : 1.6);

    if (this.action === 'walking') {
      this.advanceMovement(deltaSeconds);
    }
    this.applyPose();
    this.syncScreenPosition();
  }

  /** Local screen position of the agent's head, for anchoring chat bubbles. */
  public get headOffsetY(): number {
    return this.figure.y - 46 * AGENT_SCALE;
  }

  private beginNextSegment(initial: boolean): void {
    const next = this.queue.shift();
    if (!next) {
      this.settleAtDestination();
      return;
    }
    this.segmentFrom = initial
      ? { x: this.tileX, y: this.tileY, level: this.level }
      : { ...this.segmentTo };
    this.segmentTo = { ...next };
    this.segmentProgress = 0;
    this.action = 'walking';
    this.seatDropY = 0;
    this.updateFacingFromSegment();
  }

  private advanceMovement(deltaSeconds: number): void {
    const distance = Math.hypot(
      this.segmentTo.x - this.segmentFrom.x,
      this.segmentTo.y - this.segmentFrom.y
    );
    const step = distance < ARRIVAL_EPSILON ? 1 : (this.speed * deltaSeconds) / distance;
    this.segmentProgress += step;

    if (this.segmentProgress >= 1) {
      this.tileX = this.segmentTo.x;
      this.tileY = this.segmentTo.y;
      this.level = this.segmentTo.level;
      if (this.queue.length > 0) {
        this.beginNextSegment(false);
      } else {
        this.settleAtDestination();
      }
      return;
    }

    this.tileX = lerp(this.segmentFrom.x, this.segmentTo.x, this.segmentProgress);
    this.tileY = lerp(this.segmentFrom.y, this.segmentTo.y, this.segmentProgress);
    this.level = lerp(this.segmentFrom.level, this.segmentTo.level, this.segmentProgress);
  }

  private settleAtDestination(): void {
    this.action = this.arrivalAction;
    if (this.arrivalFacing) {
      this.facing = this.arrivalFacing;
    }
    this.seatDropY = this.arrivalAction === 'sitting' ? this.arrivalSeatDropY : 0;
  }

  private updateFacingFromSegment(): void {
    const deltaScreenX =
      this.segmentTo.x - this.segmentTo.y - (this.segmentFrom.x - this.segmentFrom.y);
    if (deltaScreenX > ARRIVAL_EPSILON) {
      this.facing = 'right';
    } else if (deltaScreenX < -ARRIVAL_EPSILON) {
      this.facing = 'left';
    }
  }

  private applyPose(): void {
    const isWalking = this.action === 'walking';
    const bob = isWalking ? Math.abs(Math.sin(this.bobPhase)) * 3 : Math.sin(this.bobPhase) * 1.2;
    this.figure.y = this.seatDropY - bob;
    this.figure.scale.x = (this.facing === 'left' ? -1 : 1) * AGENT_SCALE;
    // Squash a touch while seated so the agent reads as "sitting".
    this.figure.scale.y = (this.action === 'sitting' ? 0.92 : 1) * AGENT_SCALE;
    this.nameplate.y = this.figure.y - 44 * AGENT_SCALE;
    this.shadow.scale.set(AGENT_SCALE * (isWalking ? 1.05 : 1), AGENT_SCALE);
  }

  private syncScreenPosition(): void {
    const screen = tileCenterToScreen(this.tileX, this.tileY, this.level);
    this.position.set(screen.x, screen.y);
    this.zIndex = depthAt(this.tileX, this.tileY, this.level, LAYER_AGENT);
  }
}
